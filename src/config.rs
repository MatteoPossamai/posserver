use crate::error::{invalid, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub monzo: Monzo,
    pub backup: Backup,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Monzo {
    pub base_url: String,
    pub links_by_user_name: BTreeMap<String, String>,
    pub links_by_user_id: BTreeMap<String, String>,
    pub max_objects: usize,
}
impl Default for Monzo {
    fn default() -> Self {
        Self {
            base_url: "https://api.monzo.com".into(),
            links_by_user_name: BTreeMap::new(),
            links_by_user_id: BTreeMap::new(),
            max_objects: 100_000,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Backup {
    pub directory: Option<PathBuf>,
    pub automatic: bool,
    pub dropbox: Option<Dropbox>,
}
impl Default for Backup {
    fn default() -> Self {
        Self {
            directory: None,
            automatic: true,
            dropbox: None,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Dropbox {
    pub content_base_url: String,
    pub api_base_url: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub app_key: Option<String>,
    pub app_secret: Option<String>,
    pub root: String,
}
impl Default for Dropbox {
    fn default() -> Self {
        Self {
            content_base_url: "https://content.dropboxapi.com".into(),
            api_base_url: "https://api.dropboxapi.com".into(),
            access_token: None,
            refresh_token: None,
            app_key: None,
            app_secret: None,
            root: "/posserver".into(),
        }
    }
}
impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(p) => serde_json::from_slice(&std::fs::read(p)?).map_err(|_| invalid()),
            None => Ok(Self::default()),
        }
    }
    pub fn directory(&self, db: &Path) -> PathBuf {
        self.backup
            .directory
            .clone()
            .unwrap_or_else(|| db.parent().unwrap_or(Path::new(".")).join("backups"))
    }
}
