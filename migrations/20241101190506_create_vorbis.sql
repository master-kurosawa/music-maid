CREATE TABLE IF NOT EXISTS vorbis_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meta_id INTEGER NOT NULL,
    key TEXT NOT NULL,
    file_ptr INTEGER NOT NULL,
    size INTEGER NOT NULL,
    value TEXT,
    FOREIGN KEY (meta_id) REFERENCES vorbis_meta(id)
);

CREATE TABLE IF NOT EXISTS vorbis_meta (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    file_ptr INTEGER NOT NULL,
    FOREIGN KEY (file_id) REFERENCES files(id)
);

