use crate::config::app_paths::AppPaths;
use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Once;

static INIT: Once = Once::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerRow {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args_json: String,
    pub enabled_apps_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    pub path: String,
    pub enabled_apps_json: String,
}

fn get_db_path() -> PathBuf {
    let paths = AppPaths::new();
    paths.ensure_data_dir(&["data"]);
    paths.data_path(&["data", "mcp_skills.db"])
}

fn get_conn() -> Result<Connection> {
    Connection::open(get_db_path())
}

pub fn init_db() {
    INIT.call_once(|| {
        if let Ok(conn) = get_conn() {
            let _ = conn.execute(
                "CREATE TABLE IF NOT EXISTS mcp_servers (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    command TEXT NOT NULL,
                    args_json TEXT NOT NULL DEFAULT '[]',
                    enabled_apps_json TEXT NOT NULL DEFAULT '{}'
                )",
                [],
            );
            let _ = conn.execute(
                "CREATE TABLE IF NOT EXISTS skills (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    enabled_apps_json TEXT NOT NULL DEFAULT '{}'
                )",
                [],
            );
        } else {
            tracing::error!("Failed to create mcp_skills.db");
        }
    });
}

pub fn get_enabled_mcps_for_agent(agent_kind: &str) -> Result<Vec<McpServerRow>> {
    let conn = get_conn()?;
    let mut stmt =
        conn.prepare("SELECT id, name, command, args_json, enabled_apps_json FROM mcp_servers")?;

    let mut rows = Vec::new();
    let mcp_iter = stmt.query_map([], |row| {
        Ok(McpServerRow {
            id: row.get(0)?,
            name: row.get(1)?,
            command: row.get(2)?,
            args_json: row.get(3)?,
            enabled_apps_json: row.get(4)?,
        })
    })?;

    for mcp in mcp_iter {
        if let Ok(m) = mcp {
            if let Ok(apps) = serde_json::from_str::<serde_json::Value>(&m.enabled_apps_json) {
                if apps
                    .get(agent_kind)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    rows.push(m);
                }
            }
        }
    }

    Ok(rows)
}

pub fn get_enabled_skills_for_agent(agent_kind: &str) -> Result<Vec<SkillRow>> {
    let conn = get_conn()?;
    let mut stmt = conn.prepare("SELECT id, name, path, enabled_apps_json FROM skills")?;

    let mut rows = Vec::new();
    let skill_iter = stmt.query_map([], |row| {
        Ok(SkillRow {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            enabled_apps_json: row.get(3)?,
        })
    })?;

    for skill in skill_iter {
        if let Ok(s) = skill {
            if let Ok(apps) = serde_json::from_str::<serde_json::Value>(&s.enabled_apps_json) {
                if apps
                    .get(agent_kind)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    rows.push(s);
                }
            }
        }
    }

    Ok(rows)
}

// For testing or manual addition from CLI later
pub fn upsert_mcp_server(row: McpServerRow) -> Result<()> {
    let conn = get_conn()?;
    conn.execute(
        "INSERT INTO mcp_servers (id, name, command, args_json, enabled_apps_json) 
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET 
         name=excluded.name, command=excluded.command, 
         args_json=excluded.args_json, enabled_apps_json=excluded.enabled_apps_json",
        params![
            row.id,
            row.name,
            row.command,
            row.args_json,
            row.enabled_apps_json
        ],
    )?;
    Ok(())
}

pub fn upsert_skill(row: SkillRow) -> Result<()> {
    let conn = get_conn()?;
    conn.execute(
        "INSERT INTO skills (id, name, path, enabled_apps_json) 
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET 
         name=excluded.name, path=excluded.path, 
         enabled_apps_json=excluded.enabled_apps_json",
        params![row.id, row.name, row.path, row.enabled_apps_json],
    )?;
    Ok(())
}

pub fn get_all_mcps() -> Result<Vec<McpServerRow>> {
    let conn = get_conn()?;
    let mut stmt =
        conn.prepare("SELECT id, name, command, args_json, enabled_apps_json FROM mcp_servers")?;
    let rows = stmt.query_map([], |row| {
        Ok(McpServerRow {
            id: row.get(0)?,
            name: row.get(1)?,
            command: row.get(2)?,
            args_json: row.get(3)?,
            enabled_apps_json: row.get(4)?,
        })
    })?;

    let mut result = Vec::new();
    for r in rows {
        if let Ok(mcp) = r {
            result.push(mcp);
        }
    }
    Ok(result)
}

pub fn delete_mcp(id: &str) -> Result<()> {
    let conn = get_conn()?;
    conn.execute("DELETE FROM mcp_servers WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn get_all_skills() -> Result<Vec<SkillRow>> {
    let conn = get_conn()?;
    let mut stmt = conn.prepare("SELECT id, name, path, enabled_apps_json FROM skills")?;
    let rows = stmt.query_map([], |row| {
        Ok(SkillRow {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            enabled_apps_json: row.get(3)?,
        })
    })?;

    let mut result = Vec::new();
    for r in rows {
        if let Ok(skill) = r {
            result.push(skill);
        }
    }
    Ok(result)
}

pub fn delete_skill(id: &str) -> Result<()> {
    let conn = get_conn()?;
    conn.execute("DELETE FROM skills WHERE id = ?1", params![id])?;
    Ok(())
}
