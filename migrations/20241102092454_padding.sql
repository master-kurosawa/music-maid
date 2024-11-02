CREATE TABLE IF NOT EXISTS padding (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    file_ptr INTEGER,
    byte_size INTEGER,
    FOREIGN KEY (file_id) REFERENCES files(id)
  );
