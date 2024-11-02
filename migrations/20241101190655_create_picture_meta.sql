CREATE TABLE IF NOT EXISTS picture_metadata(
        file_id INTEGER NOT NULL,
        file_ptr INTEGER,
        picture_type INTEGER NOT NULL,
        mime TEXT NOT NULL,
        description TEXT NOT NULL,
        width INTEGER NOT NULL,
        height INTEGER NOT NULL,
        color_depth INTEGER NOT NULL,
        indexed_color_number INTEGER NOT NULL,
        size INTEGER NOT NULL,
        FOREIGN KEY (file_id) REFERENCES files(id)
);

