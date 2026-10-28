use dgos_types::{DgosError, DgosResult, ExecutionProvenance, UsageLog};
use sea_orm::*;
use uuid::Uuid;

use crate::entities::usage_logs;

/// Usage log repository for tracking provider usage
pub struct UsageLogRepository {
    db: DatabaseConnection,
}

impl UsageLogRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Create a usage log entry
    pub async fn create(
        &self,
        provider_id: &str,
        model_id: &str,
        user_id: &str,
        request_tokens: u32,
        response_tokens: u32,
        latency_ms: u32,
        success: bool,
        error_code: Option<String>,
    ) -> DgosResult<UsageLog> {
        self.create_with_provenance(
            provider_id,
            model_id,
            user_id,
            request_tokens,
            response_tokens,
            latency_ms,
            success,
            error_code,
            None,
        )
        .await
    }

    pub async fn create_with_provenance(
        &self,
        provider_id: &str,
        model_id: &str,
        user_id: &str,
        request_tokens: u32,
        response_tokens: u32,
        latency_ms: u32,
        success: bool,
        error_code: Option<String>,
        provenance: Option<&ExecutionProvenance>,
    ) -> DgosResult<UsageLog> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        let log = usage_logs::ActiveModel {
            id: Set(id.clone()),
            provider_id: Set(provider_id.to_string()),
            model_id: Set(model_id.to_string()),
            user_id: Set(user_id.to_string()),
            request_tokens: Set(request_tokens as i32),
            response_tokens: Set(response_tokens as i32),
            total_tokens: Set((request_tokens + response_tokens) as i32),
            latency_ms: Set(latency_ms as i32),
            success: Set(success),
            error_code: Set(error_code.clone()),
            caller: Set(provenance.map(|p| format!("{:?}", p.caller).to_lowercase())),
            request_id: Set(provenance.map(|p| p.request_id.clone())),
            created_at: Set(now),
        };

        usage_logs::Entity::insert(log)
            .exec(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        Ok(UsageLog {
            id,
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            user_id: user_id.to_string(),
            request_tokens,
            response_tokens,
            total_tokens: request_tokens + response_tokens,
            latency_ms,
            success,
            error_code,
            caller: provenance.map(|p| format!("{:?}", p.caller).to_lowercase()),
            request_id: provenance.map(|p| p.request_id.clone()),
            created_at: now,
        })
    }

    /// List usage logs for a user
    pub async fn list(&self, user_id: &str, limit: Option<usize>) -> DgosResult<Vec<UsageLog>> {
        let query = usage_logs::Entity::find()
            .filter(usage_logs::Column::UserId.eq(user_id))
            .order_by_desc(usage_logs::Column::CreatedAt)
            .limit(limit.unwrap_or(100).min(100) as u64);

        let logs = query
            .all(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        Ok(logs
            .into_iter()
            .map(|log| UsageLog {
                id: log.id,
                provider_id: log.provider_id,
                model_id: log.model_id,
                user_id: log.user_id,
                request_tokens: log.request_tokens as u32,
                response_tokens: log.response_tokens as u32,
                total_tokens: log.total_tokens as u32,
                latency_ms: log.latency_ms as u32,
                success: log.success,
                error_code: log.error_code,
                caller: log.caller,
                request_id: log.request_id,
                created_at: log.created_at,
            })
            .collect())
    }

    /// Get usage statistics for a provider
    pub async fn get_provider_stats(&self, provider_id: &str) -> DgosResult<ProviderStats> {
        let logs = usage_logs::Entity::find()
            .filter(usage_logs::Column::ProviderId.eq(provider_id))
            .all(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        let total_requests = logs.len();
        let successful_requests = logs.iter().filter(|log| log.success).count();
        let total_tokens: i32 = logs.iter().map(|log| log.total_tokens).sum();
        let avg_latency = if total_requests > 0 {
            logs.iter().map(|log| log.latency_ms).sum::<i32>() / total_requests as i32
        } else {
            0
        };

        Ok(ProviderStats {
            total_requests,
            successful_requests,
            failed_requests: total_requests - successful_requests,
            total_tokens: total_tokens as u32,
            avg_latency_ms: avg_latency as u32,
        })
    }
}

#[derive(Debug)]
pub struct ProviderStats {
    pub total_requests: usize,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub total_tokens: u32,
    pub avg_latency_ms: u32,
}
