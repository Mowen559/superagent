use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};

pub struct EpisodicMemoryStore {
    db_path: PathBuf,
}

impl EpisodicMemoryStore {
    pub fn new(brain_dir: PathBuf) -> Self {
        let db_path = brain_dir.join("episodic_memory.db");
        Self::init_db(&db_path).unwrap_or_else(|e| eprintln!("Failed to init episodic db: {}", e));
        Self { db_path }
    }
    
    fn init_db(db_path: &PathBuf) -> Result<(), String> {
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;").unwrap_or(());
        conn.execute(
            "CREATE TABLE IF NOT EXISTS skills (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_text TEXT NOT NULL,
                resolution_trace TEXT NOT NULL,
                vector BLOB NOT NULL
            )",
            [],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn record_skill(&self, issue: &str, resolution_trace: &str) -> Result<(), String> {
        let vector = tokio::task::block_in_place(|| -> Result<Vec<f32>, String> {
            let mut model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
                .map_err(|e| e.to_string())?;
            let mut embeddings = model.embed(vec![issue], None).map_err(|e| e.to_string())?;
            embeddings.into_iter().next().ok_or_else(|| "Embedding model returned empty results".to_string())
        })?;
        
        // Convert f32 vector to byte array
        let mut vector_bytes = Vec::with_capacity(vector.len() * 4);
        for v in vector {
            vector_bytes.extend_from_slice(&v.to_le_bytes());
        }

        let conn = Connection::open(&self.db_path).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO skills (issue_text, resolution_trace, vector) VALUES (?1, ?2, ?3)",
            rusqlite::params![issue, resolution_trace, vector_bytes],
        ).map_err(|e| e.to_string())?;
        
        Ok(())
    }

    pub async fn recall_skills(&self, current_context: &str) -> Result<Vec<String>, String> {
        let query_vec = tokio::task::block_in_place(|| -> Result<Vec<f32>, String> {
            let mut model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
                .map_err(|e| e.to_string())?;
            let mut embeddings = model.embed(vec![current_context], None).map_err(|e| e.to_string())?;
            embeddings.into_iter().next().ok_or_else(|| "Embedding model returned empty results".to_string())
        })?;

        let conn = Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
            
        let mut stmt = conn.prepare("SELECT resolution_trace, vector FROM skills").map_err(|e| e.to_string())?;
        
        let hits: Result<Vec<(String, f32)>, _> = stmt.query_map([], |row| {
            let trace: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            
            // Reconstruct f32 vector
            let mut db_vec = Vec::with_capacity(blob.len() / 4);
            for chunk in blob.chunks_exact(4) {
                db_vec.push(f32::from_le_bytes(chunk.try_into().unwrap()));
            }
            
            // Cosine similarity
            let mut dot = 0.0;
            let mut ma = 0.0;
            let mut mb = 0.0;
            for (a, b) in query_vec.iter().zip(db_vec.iter()) {
                dot += a * b;
                ma += a * a;
                mb += b * b;
            }
            let sim = if ma > 0.0 && mb > 0.0 { dot / (ma.sqrt() * mb.sqrt()) } else { 0.0 };
            
            Ok((trace, sim))
        }).map_err(|e| e.to_string())?.collect();
        
        let mut hits = hits.map_err(|e| e.to_string())?;
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        
        Ok(hits.into_iter().filter(|(_, sim)| *sim > 0.6).take(3).map(|(t, _)| t).collect())
    }
}

