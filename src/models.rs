use serde::{Deserialize, Serialize};

pub const MAX_AMOUNT: i64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FxRate {
    pub from: String,
    pub to: String,
    pub rate: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub language: String,
    pub timezone: String,
    pub reporting_currency: String,
    pub category_profile: String,
    pub version: i64,
    pub rates: Vec<FxRate>,
    pub monzo_linked: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub user_id: String,
    pub occurred_at: String,
    pub category_id: String,
    pub issuer: String,
    pub amount_minor: i64,
    pub currency: String,
    pub source: String,
    pub version: i64,
    pub monzo_transaction_id: Option<String>,
}
#[derive(Clone, Default, Serialize)]
pub struct Summary {
    pub expense_minor: i64,
    pub income_minor: i64,
    pub net_minor: i64,
    pub count: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub en: String,
    pub it: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub revision: i64,
    pub schema_version: i64,
    pub sha256: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
}
