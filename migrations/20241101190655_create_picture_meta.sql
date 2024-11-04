CREATE TABLE IF NOT EXISTS picture_metadata(
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        file_id INTEGER,
        file_ptr INTEGER NOT NULL,
        picture_type INTEGER NOT NULL,
        mime TEXT NOT NULL,
        description TEXT NOT NULL,
        width INTEGER NOT NULL,
        height INTEGER NOT NULL,
        color_depth INTEGER NOT NULL,
        indexed_color_number INTEGER NOT NULL,
        size INTEGER NOT NULL,
        vorbis_comment BOOLEAN NOT NULL,
        FOREIGN KEY (file_id) REFERENCES files(id)
);

