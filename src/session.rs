use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Metadata tracked by the session store for each session
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMetadata {
    created_at: SystemTime,
    last_used: SystemTime,
}

/// Internal wrapper for session data with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionEntry<T> {
    data: T,
    metadata: SessionMetadata,
}

/// Generic session store that handles persistence and lifecycle management
pub struct SessionStore<T> {
    sessions: Arc<Mutex<HashMap<String, SessionEntry<T>>>>,
    storage_dir: PathBuf,
}

impl<T> SessionStore<T>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone + Default,
{
    /// Create a new session store with the given storage directory
    pub fn new(storage_dir: impl AsRef<Path>) -> Result<Self> {
        let storage_dir = storage_dir.as_ref().to_path_buf();

        // Ensure storage directory exists
        if let Some(parent) = storage_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir_all(&storage_dir)?;

        let store = Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            storage_dir,
        };

        // Load existing sessions from disk
        store.load_from_disk()?;

        Ok(store)
    }

    /// Get session data, creating a new session if it doesn't exist
    pub fn get_or_create(&self, session_id: &str) -> Result<T> {
        let mut sessions = self.sessions.lock().unwrap();

        match sessions.get_mut(session_id) {
            Some(entry) => {
                // Update last_used timestamp
                entry.metadata.last_used = SystemTime::now();
                self.save_session_to_disk(session_id, entry)?;
                Ok(entry.data.clone())
            }
            None => {
                // Create new session
                let now = SystemTime::now();
                let entry = SessionEntry {
                    data: T::default(),
                    metadata: SessionMetadata {
                        created_at: now,
                        last_used: now,
                    },
                };

                sessions.insert(session_id.to_string(), entry.clone());
                drop(sessions); // Release lock before I/O

                self.save_session_to_disk(session_id, &entry)?;
                Ok(entry.data)
            }
        }
    }

    /// Update session data
    pub fn set(&self, session_id: &str, data: T) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        let now = SystemTime::now();

        let entry = SessionEntry {
            data,
            metadata: SessionMetadata {
                created_at: sessions
                    .get(session_id)
                    .map(|e| e.metadata.created_at)
                    .unwrap_or(now),
                last_used: now,
            },
        };

        sessions.insert(session_id.to_string(), entry.clone());
        drop(sessions); // Release lock before I/O

        self.save_session_to_disk(session_id, &entry)?;
        Ok(())
    }

    /// Remove a session
    pub fn remove(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(session_id);
        drop(sessions); // Release lock before I/O

        let session_file = self.session_file_path(session_id);
        if session_file.exists() {
            fs::remove_file(session_file)?;
        }
        Ok(())
    }

    /// List all session IDs
    pub fn list_sessions(&self) -> Vec<String> {
        let sessions = self.sessions.lock().unwrap();
        sessions.keys().cloned().collect()
    }

    /// Load all sessions from disk
    fn load_from_disk(&self) -> Result<()> {
        if !self.storage_dir.exists() {
            return Ok(());
        }

        let mut sessions = self.sessions.lock().unwrap();

        for entry in fs::read_dir(&self.storage_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
                if let Some(session_id) = path.file_stem().and_then(|s| s.to_str()) {
                    match self.load_session_from_disk(session_id) {
                        Ok(session_entry) => {
                            sessions.insert(session_id.to_string(), session_entry);
                        }
                        Err(e) => {
                            eprintln!("Failed to load session {session_id}: {e}");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a single session from disk
    fn load_session_from_disk(&self, session_id: &str) -> Result<SessionEntry<T>> {
        let session_file = self.session_file_path(session_id);
        let content = fs::read_to_string(session_file)?;
        let entry: SessionEntry<T> = serde_json::from_str(&content)?;
        Ok(entry)
    }

    /// Save a single session to disk
    fn save_session_to_disk(&self, session_id: &str, entry: &SessionEntry<T>) -> Result<()> {
        let session_file = self.session_file_path(session_id);
        let content = serde_json::to_string_pretty(entry)?;
        fs::write(session_file, content)?;
        Ok(())
    }

    /// Get the file path for a session
    fn session_file_path(&self, session_id: &str) -> PathBuf {
        self.storage_dir.join(format!("{session_id}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    struct TestSessionData {
        value: i32,
    }

    #[test]
    fn test_session_store_basic_operations() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let store = SessionStore::<TestSessionData>::new(temp_dir.path())?;

        // Test get_or_create with new session
        let data = store.get_or_create("test_session")?;
        assert_eq!(data, TestSessionData::default());

        // Test set
        let new_data = TestSessionData { value: 42 };
        store.set("test_session", new_data.clone())?;

        // Test get existing session
        let retrieved_data = store.get_or_create("test_session")?;
        assert_eq!(retrieved_data, new_data);

        Ok(())
    }

    #[test]
    fn test_session_persistence() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_data = TestSessionData { value: 42 };

        // Create a session and save data
        {
            let store = SessionStore::<TestSessionData>::new(temp_dir.path())?;
            store.set("persistent_session", test_data.clone())?;
        }

        // Create a new store instance and verify data persisted
        {
            let store = SessionStore::<TestSessionData>::new(temp_dir.path())?;
            let retrieved_data = store.get_or_create("persistent_session")?;
            assert_eq!(retrieved_data, test_data);
        }

        Ok(())
    }
}
