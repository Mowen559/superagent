//! 会话状态快照与断点续跑持久化存储 (Session Checkpointing & Resume Store)
//!
//! 在本地轻量级 SQLite 中记录所有长任务的断点快照：
//! 1. 任务终极目标 (Goal)
//! 2. 计划步骤与当前执行步数 (TaskPlan: CurrentStep / TotalSteps)
//! 3. 会话状态 (InProgress, Interrupted, Completed)
//! 4. 支持 CLI 随时列出并一键无损续跑，彻底消除意外中断引起的“失忆”。

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Once;
use chrono::Utc;
use crate::config::app_paths::AppPaths;

static INIT_CHECKPOINT_DB: Once = Once::new();

fn get_checkpoint_db_path() -> PathBuf {
    AppPaths::new().data_path(&["data", "checkpoints.db"])
}

/// 确保 Checkpoint 数据库与表结构初始化
pub fn init_checkpoint_db() {
    INIT_CHECKPOINT_DB.call_once(|| {
        let db_path = get_checkpoint_db_path();
        let paths = AppPaths::new();
        paths.ensure_data_dir(&["data"]);

        if let Ok(conn) = Connection::open(&db_path) {
            let _ = conn.execute(
                "CREATE TABLE IF NOT EXISTS session_checkpoints (
                    session_id TEXT PRIMARY KEY,
                    goal TEXT NOT NULL,
                    status TEXT NOT NULL,
                    current_step INTEGER NOT NULL DEFAULT 0,
                    total_steps INTEGER NOT NULL DEFAULT 0,
                    step_description TEXT NOT NULL DEFAULT '',
                    context_snapshot TEXT,
                    created_at DATETIME NOT NULL,
                    updated_at DATETIME NOT NULL
                )",
                [],
            );
        } else {
            tracing::error!("Failed to create checkpoints.db at {:?}", db_path);
        }
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpointRecord {
    pub session_id: String,
    pub goal: String,
    pub status: String,
    pub current_step: usize,
    pub total_steps: usize,
    pub step_description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_snapshot: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct SessionCheckpointStore;

impl SessionCheckpointStore {
    /// 记录或原子更新会话断点状态
    pub fn save_checkpoint(
        session_id: &str,
        goal: &str,
        status: &str,
        current_step: usize,
        total_steps: usize,
        step_description: &str,
        context_json: Option<&str>,
    ) -> Result<(), String> {
        init_checkpoint_db();
        let db_path = get_checkpoint_db_path();
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| format!("Failed to open checkpoints.db: {}", e))?;

        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO session_checkpoints (
                session_id, goal, status, current_step, total_steps, step_description, context_snapshot, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(session_id) DO UPDATE SET
                goal = excluded.goal,
                status = excluded.status,
                current_step = excluded.current_step,
                total_steps = excluded.total_steps,
                step_description = excluded.step_description,
                context_snapshot = COALESCE(excluded.context_snapshot, session_checkpoints.context_snapshot),
                updated_at = excluded.updated_at",
            rusqlite::params![
                session_id,
                goal,
                status,
                current_step as i64,
                total_steps as i64,
                step_description,
                context_json,
                now,
                now,
            ],
        ).map_err(|e| format!("Failed to upsert session checkpoint: {}", e))?;

        Ok(())
    }

    /// 获取特定会话的断点记录（包含完整上下文快照）
    pub fn get_checkpoint(session_id: &str) -> Result<Option<SessionCheckpointRecord>, String> {
        init_checkpoint_db();
        let db_path = get_checkpoint_db_path();
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Failed to open checkpoints.db: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT session_id, goal, status, current_step, total_steps, step_description, context_snapshot, created_at, updated_at
             FROM session_checkpoints WHERE session_id = ?1"
        ).map_err(|e| e.to_string())?;

        let mut rows = stmt.query(rusqlite::params![session_id]).map_err(|e| e.to_string())?;

        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            Ok(Some(SessionCheckpointRecord {
                session_id: row.get(0).unwrap_or_default(),
                goal: row.get(1).unwrap_or_default(),
                status: row.get(2).unwrap_or_default(),
                current_step: row.get::<_, i64>(3).unwrap_or(0).max(0) as usize,
                total_steps: row.get::<_, i64>(4).unwrap_or(0).max(0) as usize,
                step_description: row.get(5).unwrap_or_default(),
                context_snapshot: row.get(6).ok(),
                created_at: row.get(7).unwrap_or_default(),
                updated_at: row.get(8).unwrap_or_default(),
            }))
        } else {
            Ok(None)
        }
    }

    /// 列出所有会话快照（按更新时间降序，不包含庞大的 context_snapshot 以保证轻量）
    pub fn list_checkpoints(limit: usize) -> Result<Vec<SessionCheckpointRecord>, String> {
        init_checkpoint_db();
        let db_path = get_checkpoint_db_path();
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Failed to open checkpoints.db: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT session_id, goal, status, current_step, total_steps, step_description, created_at, updated_at
             FROM session_checkpoints
             ORDER BY updated_at DESC
             LIMIT ?1"
        ).map_err(|e| e.to_string())?;

        let iter = stmt.query_map([limit as i64], |row| {
            Ok(SessionCheckpointRecord {
                session_id: row.get(0)?,
                goal: row.get(1)?,
                status: row.get(2)?,
                current_step: row.get::<_, i64>(3)?.max(0) as usize,
                total_steps: row.get::<_, i64>(4)?.max(0) as usize,
                step_description: row.get(5)?,
                context_snapshot: None,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        }).map_err(|e| e.to_string())?;

        let mut res = Vec::new();
        for r in iter {
            if let Ok(rec) = r {
                res.push(rec);
            }
        }
        Ok(res)
    }

    /// 获取最近一个未完成或被中断的会话
    pub fn get_last_interrupted_session() -> Result<Option<SessionCheckpointRecord>, String> {
        let list = Self::list_checkpoints(5)?;
        for rec in list {
            if rec.status == "in_progress" || rec.status == "interrupted" {
                return Ok(Some(rec));
            }
        }
        Ok(None)
    }

    /// 唤醒并恢复断点会话，将状态更新为 in_progress
    pub fn resume_checkpoint(session_id: &str) -> Result<SessionCheckpointRecord, String> {
        init_checkpoint_db();
        let db_path = get_checkpoint_db_path();
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| format!("Failed to open checkpoints.db: {}", e))?;

        let now = Utc::now().to_rfc3339();
        let rows_affected = conn.execute(
            "UPDATE session_checkpoints SET status = 'in_progress', updated_at = ?1 WHERE session_id = ?2",
            rusqlite::params![now, session_id],
        ).map_err(|e| e.to_string())?;

        if rows_affected == 0 {
            return Err(format!("Session {} not found", session_id));
        }

        Self::get_checkpoint(session_id)?.ok_or_else(|| format!("Session {} disappeared", session_id))
    }

    /// 标记会话已圆满完成
    pub fn mark_completed(session_id: &str) -> Result<(), String> {
        init_checkpoint_db();
        let db_path = get_checkpoint_db_path();
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| format!("Failed to open checkpoints.db: {}", e))?;

        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE session_checkpoints SET status = 'completed', updated_at = ?1 WHERE session_id = ?2",
            rusqlite::params![now, session_id],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// 删除特定会话快照
    pub fn delete_checkpoint(session_id: &str) -> Result<(), String> {
        init_checkpoint_db();
        let db_path = get_checkpoint_db_path();
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| format!("Failed to open checkpoints.db: {}", e))?;

        conn.execute(
            "DELETE FROM session_checkpoints WHERE session_id = ?1",
            rusqlite::params![session_id],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_lifecycle() {
        let test_session_id = format!("test_sess_{}", uuid::Uuid::new_v4());
        
        // 1. 保存初次断点
        let res = SessionCheckpointStore::save_checkpoint(
            &test_session_id,
            "逆向目标网页并抓取海阔视界规则",
            "interrupted",
            2,
            5,
            "正在使用 spider_runner 运行本地沙箱测试",
            Some("{\"state\": \"ok\"}"),
        );
        assert!(res.is_ok());

        // 2. 查询断点
        let loaded = SessionCheckpointStore::get_checkpoint(&test_session_id).unwrap();
        assert!(loaded.is_some());
        let rec = loaded.unwrap();
        assert_eq!(rec.session_id, test_session_id);
        assert_eq!(rec.current_step, 2);
        assert_eq!(rec.total_steps, 5);
        assert_eq!(rec.status, "interrupted");
        assert_eq!(rec.context_snapshot, Some("{\"state\": \"ok\"}".to_string()));

        // 3. 一键断点续跑 (Resume)
        let resumed = SessionCheckpointStore::resume_checkpoint(&test_session_id).unwrap();
        assert_eq!(resumed.status, "in_progress");

        // 4. 标记完成
        let comp_res = SessionCheckpointStore::mark_completed(&test_session_id);
        assert!(comp_res.is_ok());

        let finished = SessionCheckpointStore::get_checkpoint(&test_session_id).unwrap().unwrap();
        assert_eq!(finished.status, "completed");

        // 5. 删除快照
        let del_res = SessionCheckpointStore::delete_checkpoint(&test_session_id);
        assert!(del_res.is_ok());
        let after_del = SessionCheckpointStore::get_checkpoint(&test_session_id).unwrap();
        assert!(after_del.is_none());
    }
}
