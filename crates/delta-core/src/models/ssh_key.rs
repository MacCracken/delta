use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshKey {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}
