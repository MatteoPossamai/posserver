"""Source packaging and generated Termux shell validation; no phone mutation."""
import importlib.machinery
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch

loader = importlib.machinery.SourceFileLoader('phone', str(Path(__file__).resolve().parents[1] / 'scripts/phone'))
spec = importlib.util.spec_from_loader(loader.name, loader)
phone = importlib.util.module_from_spec(spec)
loader.exec_module(phone)


class DeploymentTests(unittest.TestCase):
    def test_archive_and_shell(self):
        with patch.object(sys, 'argv', ['phone', 'deploy', '--bind', '100.108.243.40:8080']), patch.object(phone, 'ssh') as ssh:
            phone.main()
        command = ssh.call_args.args[1]
        subprocess.run(['bash', '-n'], input=command.encode(), check=True)
        with tarfile.open(fileobj=io.BytesIO(ssh.call_args.kwargs['input']), mode='r:gz') as tar:
            names = tar.getnames()
        self.assertIn('src/main.rs', names)
        self.assertIn('docs/categories.json', names)
        self.assertFalse(any(name.startswith(('data/', 'target/', 'tests/')) for name in names))
        self.assertFalse(any('vault' in name or name.endswith('.sqlite') for name in names))

    def test_failed_build_preserves_installed_release_and_data(self):
        with patch.object(sys, 'argv', ['phone', 'deploy']), patch.object(phone, 'ssh') as ssh:
            phone.main()
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory)
            binaries = prefix / 'bin'
            binaries.mkdir()
            for tool in ['cargo', 'clang', 'sv', 'curl']:
                script = binaries / tool
                script.write_text('#!/bin/sh\nexit 11\n' if tool == 'cargo' else '#!/bin/sh\nexit 0\n')
                script.chmod(0o700)
            app = prefix / 'apps/posserver'
            old = app / 'releases/old'
            old.mkdir(parents=True)
            (app / 'current').symlink_to(old)
            data = prefix / 'data/posserver'
            data.mkdir(parents=True)
            (data / 'database.sqlite').write_bytes(b'existing authoritative data')
            (data / 'config.json').write_text('{"backup":{"automatic":false}}')
            env = dict(os.environ, PREFIX=str(prefix), PATH=str(binaries) + ':' + os.environ['PATH'])
            result = subprocess.run(['bash', '-c', ssh.call_args.args[1]], input=ssh.call_args.kwargs['input'], env=env)
            self.assertEqual(result.returncode, 11)
            self.assertEqual((app / 'current').resolve(), old)
            self.assertEqual((data / 'database.sqlite').read_bytes(), b'existing authoritative data')
            self.assertEqual((data / 'config.json').read_text(), '{"backup":{"automatic":false}}')
            self.assertFalse((prefix / 'var/service/posserver').exists())

    def test_all_control_shells(self):
        for action in ['start', 'stop', 'restart', 'status', 'logs', 'rollback']:
            with self.subTest(action=action), patch.object(sys, 'argv', ['phone', action]), patch.object(phone, 'ssh') as ssh:
                phone.main()
                subprocess.run(['bash', '-n'], input=ssh.call_args.args[1].encode(), check=True)

    def test_configure_streams_json_without_shell_secret(self):
        with tempfile.TemporaryDirectory() as directory:
            vault = Path(directory) / 'config.vault'
            vault.write_bytes(b'$ANSIBLE_VAULT;1.1;AES256\nsynthetic')
            secret = b'{"backup":{"dropbox":{"app_secret":"synthetic-secret"}}}'
            with patch.object(sys, 'argv', ['phone', 'configure', '--vault', str(vault)]), patch.object(phone.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, secret)), patch.object(phone, 'ssh') as ssh:
                phone.main()
            self.assertEqual(ssh.call_args.kwargs['input'], secret)
            command = ssh.call_args.args[1]
            self.assertNotIn('synthetic-secret', command)
            subprocess.run(['bash', '-n'], input=command.encode(), check=True)

    def test_plaintext_configuration_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / 'config.json'
            config.write_text('{}')
            with patch.object(sys, 'argv', ['phone', 'configure', '--vault', str(config)]), patch.object(phone, 'ssh') as ssh:
                with self.assertRaises(SystemExit):
                    phone.main()
                ssh.assert_not_called()

    def test_invalid_address_never_connects(self):
        with patch.object(sys, 'argv', ['phone', 'deploy', '--bind', '$(touch /tmp/bad):80']), patch.object(phone, 'ssh') as ssh:
            with self.assertRaises(SystemExit):
                phone.main()
            ssh.assert_not_called()


if __name__ == '__main__':
    unittest.main()
