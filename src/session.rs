use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// Metadata tracked by the session store for each session
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMetadata {
    created_at: SystemTime,
    last_used: SystemTime,
}

/// Internal wrapper for session data with metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SessionEntry<T> {
    data: T,
    metadata: SessionMetadata,
}
impl<T> SessionEntry<T> {
    fn update_last_used(&mut self) {
        self.metadata.last_used = SystemTime::now();
    }
}

/// Generic session store that handles persistence and lifecycle management
#[derive(Debug, Clone)]
pub struct SessionStore<T> {
    sessions: Arc<RwLock<HashMap<String, SessionEntry<T>>>>,
    storage_path: PathBuf,
}

impl Default for SessionMetadata {
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            created_at: now,
            last_used: now,
        }
    }
}

impl<T> SessionStore<T>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone + Default,
{
    /// Create a new session store with the given storage directory
    pub fn new(storage_path: impl AsRef<Path>) -> Result<Self> {
        let storage_path = storage_path.as_ref().to_path_buf();

        // Ensure storage directory exists
        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        OpenOptions::new()
            .append(true)
            .create(true)
            .open(&storage_path)
            .map_err(|_| anyhow!("could not open {}", storage_path.to_string_lossy()))?;

        let store = Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            storage_path,
        };

        // Load existing sessions from disk
        store.load()?;

        Ok(store)
    }

    /// Get session data, creating a new session if it doesn't exist
    pub fn get_or_create(&self, session_id: &str) -> Result<T> {
        let data;
        {
            let mut sessions = self.sessions.write().unwrap();

            let entry = sessions
                .entry(session_id.to_string())
                .and_modify(|e| e.update_last_used())
                .or_default();
            data = entry.data.clone();
        }

        self.save()?;
        Ok(data)
    }

    pub fn update(&self, session_id: &str, fun: impl FnOnce(&mut T)) -> Result<()> {
        {
            let mut sessions = self.sessions.write().unwrap();

            let entry = sessions
                .entry(session_id.to_string())
                .and_modify(|e| e.update_last_used())
                .or_default();

            fun(&mut entry.data);
        }

        self.save()?;
        Ok(())
    }

    /// Update session data
    pub fn set(&self, session_id: &str, data: T) -> Result<()> {
        self.update(session_id, |existing| *existing = data)
    }

    /// Load sessions from disk
    fn load(&self) -> Result<()> {
        let contents = std::fs::read_to_string(&self.storage_path)?;
        if let Ok(sessions) = serde_json::from_str(&contents) {
            *self.sessions.write().unwrap() = sessions;
        }
        Ok(())
    }

    /// Save sessions to disk
    fn save(&self) -> Result<()> {
        let contents = serde_json::to_string_pretty(&*self.sessions.read().unwrap())?;
        std::fs::write(&self.storage_path, &contents)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
    struct TestSessionData {
        value: i32,
    }

    #[test]
    fn test_session_store_basic_operations() -> Result<()> {
        let storage_path = NamedTempFile::new()?;
        let store = SessionStore::<TestSessionData>::new(storage_path.path())?;

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
        let storage_path = NamedTempFile::new()?;
        let test_data = TestSessionData { value: 42 };

        // Create a session and save data
        {
            let store = SessionStore::<TestSessionData>::new(storage_path.path())?;
            store.set("persistent_session", test_data.clone())?;
        }

        // Create a new store instance and verify data persisted
        {
            let store = SessionStore::<TestSessionData>::new(storage_path.path())?;
            let retrieved_data = store.get_or_create("persistent_session")?;
            assert_eq!(retrieved_data, test_data);
        }

        // update data
        {
            let store = SessionStore::<TestSessionData>::new(storage_path.path())?;
            store.update("persistent_session", |x| x.value = 10)?;
        }

        let store = SessionStore::<TestSessionData>::new(storage_path.path())?;
        let retrieved_data = store.get_or_create("persistent_session")?;
        assert_eq!(retrieved_data.value, 10);

        Ok(())
    }
}
