use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LfsObject {
    pub id: Uuid,
    pub repo_id: Uuid,
    pub oid: String,
    pub size: i64,
    pub created_at: DateTime<Utc>,
}
