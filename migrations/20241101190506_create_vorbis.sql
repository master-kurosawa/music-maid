CREATE TABLE IF NOT EXISTS vorbis_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meta_id INTEGER NOT NULL,
    key TEXT NOT NULL,
    file_ptr INTEGER NOT NULL,
    last_ogg_header_ptr INTEGER,
    size INTEGER NOT NULL,
    value TEXT,
    blob_hash VARCHAR(32),
    FOREIGN KEY (meta_id) REFERENCES vorbis_meta(id),
    FOREIGN KEY (blob_hash) REFERENCES vorbis_blobs(hash)
);

CREATE TABLE IF NOT EXISTS vorbis_meta (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    file_ptr INTEGER NOT NULL,
    end_ptr INTEGER NOT NULL,
    vendor TEXT NOT NULL,
    comment_amount_ptr INTEGER NOT NULL,
    FOREIGN KEY (file_id) REFERENCES files(id)
);

CREATE TABLE IF NOT EXISTS vorbis_blobs (
  hash VARCHAR(32) PRIMARY KEY,
  value BLOB,
  file_path TEXT
);
