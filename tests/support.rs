//! Black-box fixtures only. No server or finance implementation lives here.
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tiny_http::{Header, Response, Server, StatusCode};

#[derive(Clone, Debug)]
pub struct Request {
    pub path: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Request {
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}
pub struct Mock {
    pub url: String,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Mock {
    pub fn new() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let replies = Arc::new(Mutex::new(VecDeque::<Reply>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (q, r, s) = (replies.clone(), requests.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                if let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap() {
                    let mut body = Vec::new();
                    request.as_reader().read_to_end(&mut body).unwrap();
                    r.lock().unwrap().push(Request {
                        path: request.url().into(),
                        method: request.method().as_str().into(),
                        headers: request
                            .headers()
                            .iter()
                            .map(|h| (h.field.as_str().to_string(), h.value.as_str().to_string()))
                            .collect(),
                        body,
                    });
                    let reply = q.lock().unwrap().pop_front().unwrap_or(Reply {
                        status: 500,
                        body: br#"{"error":"unexpected mock request"}"#.to_vec(),
                    });
                    let response = Response::from_data(reply.body)
                        .with_status_code(StatusCode(reply.status))
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        );
                    let _ = request.respond(response);
                }
            }
        });
        Self {
            url,
            replies,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    pub fn reply(&self, status: u16, body: Value) {
        self.raw(status, serde_json::to_vec(&body).unwrap());
    }
    pub fn raw(&self, status: u16, body: Vec<u8>) {
        self.replies
            .lock()
            .unwrap()
            .push_back(Reply { status, body });
    }
    pub fn page(&self, rows: Vec<Value>) {
        self.reply(200, json!({"transactions":rows}));
    }
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
    pub fn remaining(&self) -> usize {
        self.replies.lock().unwrap().len()
    }
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(w) = self.worker.take() {
            w.join().unwrap();
        }
    }
}

pub struct App {
    pub dir: TempDir,
    pub db: PathBuf,
    pub config: PathBuf,
    pub url: String,
    pub client: Client,
    pub monzo: Mock,
    pub dropbox: Mock,
    bin: PathBuf,
    child: Option<Child>,
}
impl App {
    pub fn new() -> Self {
        let bin=PathBuf::from(std::env::var_os("POSSERVER_BIN").expect("No implementation supplied. Set POSSERVER_BIN to an absolute path to the future posserver executable; this is an expected red test, not a skip."));
        assert!(
            bin.is_absolute() && bin.is_file(),
            "POSSERVER_BIN must be an existing absolute executable path"
        );
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("database.sqlite");
        let config = dir.path().join("config.json");
        let monzo = Mock::new();
        let dropbox = Mock::new();
        fs::write(&config,serde_json::to_vec(&json!({"monzo":{"base_url":monzo.url,"links_by_user_name":{"Matteo":"acc_test"}},"backup":{"directory":dir.path().join("backups"),"automatic":false,"dropbox":{"content_base_url":dropbox.url,"api_base_url":dropbox.url,"access_token":"synthetic-dropbox-token","root":"/posserver"}}})).unwrap()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut app = Self {
            dir,
            db,
            config,
            url: format!("http://127.0.0.1:{port}"),
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            monzo,
            dropbox,
            bin,
            child: None,
        };
        app.start();
        app
    }
    pub fn start(&mut self) {
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("server.log"))
            .unwrap();
        self.child = Some(
            Command::new(&self.bin)
                .args(["serve", "--db"])
                .arg(&self.db)
                .arg("--bind")
                .arg(self.url.strip_prefix("http://").unwrap())
                .arg("--config")
                .arg(&self.config)
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        );
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(r) = self.client.get(format!("{}/healthz", self.url)).send() {
                if r.status().as_u16() == 200 {
                    break;
                }
            }
            let exited = self.child.as_mut().unwrap().try_wait().unwrap().is_some();
            assert!(
                !exited && Instant::now() < until,
                "server failed startup: {}",
                fs::read_to_string(self.dir.path().join("server.log")).unwrap()
            );
            thread::sleep(Duration::from_millis(30));
        }
    }
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    pub fn api(&self, method: &str, path: &str, body: Option<Value>, status: u16) -> Value {
        let mut req = self.client.request(
            method.parse().unwrap(),
            format!("{}/api/v1{}", self.url, path),
        );
        if let Some(b) = body {
            req = req.json(&b);
        }
        let r = req.send().unwrap();
        let actual = r.status().as_u16();
        let text = r.text().unwrap();
        assert_eq!(actual, status, "{method} {path}: {text}");
        serde_json::from_str(&text).expect("API must return JSON")
    }
    pub fn user(&self, name: &str, profile: &str, tz: &str) -> Value {
        self.api("POST","/users",Some(json!({"name":name,"language":if profile=="dad"{"it"}else{"en"},"timezone":tz,"reporting_currency":if profile=="dad"{"EUR"}else{"GBP"},"category_profile":profile})),201)["user"].clone()
    }
    pub fn create(
        &self,
        u: &str,
        date: &str,
        amount: i64,
        category: &str,
        currency: &str,
    ) -> Value {
        self.api(
            "POST",
            &format!("/users/{u}/transactions"),
            Some(manual(date, amount, category, currency)),
            201,
        )["transaction"]
            .clone()
    }
    pub fn rows(&self, u: &str) -> Vec<Value> {
        self.api(
            "GET",
            &format!("/users/{u}/transactions?limit=100"),
            None,
            200,
        )["transactions"]
            .as_array()
            .unwrap()
            .clone()
    }
    pub fn import(&self, u: &str, since: Option<&str>, status: u16) -> Value {
        let mut b = json!({"access_token":"synthetic-monzo-token"});
        if let Some(s) = since {
            b["since"] = json!(s);
        }
        self.api(
            "POST",
            &format!("/users/{u}/imports/monzo"),
            Some(b),
            status,
        )
    }
    pub fn report(&self, u: &str, month: &str, currency: &str, status: u16) -> Value {
        self.api(
            "GET",
            &format!("/users/{u}/reports/month?month={month}&currency={currency}"),
            None,
            status,
        )
    }
    pub fn status(&self) -> Value {
        self.api("GET", "/backups/status", None, 200)
    }
    pub fn cli(&self, args: &[&str]) -> Output {
        Command::new(&self.bin).args(args).output().unwrap()
    }
    pub fn csv(&self, content: &str) -> PathBuf {
        let p = self.dir.path().join("legacy.csv");
        fs::write(&p, content).unwrap();
        p
    }
    pub fn migrate(&self, u: &str, file: &Path, extra: &[&str]) -> Output {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["migrate-csv", "--db"])
            .arg(&self.db)
            .args(["--user", u, "--file"])
            .arg(file)
            .args(extra);
        cmd.output().unwrap()
    }
    pub fn backup(&self, provider: &str) -> Output {
        Command::new(&self.bin)
            .arg(format!("backup-{provider}"))
            .arg("--db")
            .arg(&self.db)
            .arg("--config")
            .arg(&self.config)
            .output()
            .unwrap()
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}
pub fn id(v: &Value) -> &str {
    v["id"].as_str().unwrap()
}
pub fn manual(date: &str, amount: i64, category: &str, currency: &str) -> Value {
    json!({"occurred_at":date,"category_id":category,"issuer":"Synthetic shop","amount_minor":amount,"currency":currency})
}
pub fn tx(id: &str, date: &str, amount: i64, issuer: &str) -> Value {
    json!({"id":id,"created":date,"category":"groceries","local_amount":amount,"local_currency":"GBP","amount":-999999,"currency":"EUR","description":format!("{issuer}  LONDON"),"counterparty":{},"settled":""})
}
pub fn output_json(o: &Output) -> Value {
    assert!(
        o.status.success(),
        "CLI failed: stdout={} stderr={}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
pub fn error(v: &Value, code: &str) {
    assert_eq!(v["error"]["code"], code);
}
