CREATE TABLE IF NOT EXISTS vorbis_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    vendor TEXT NOT NULL,
    title TEXT NOT NULL,
    version TEXT NOT NULL,
    album TEXT NOT NULL,
    tracknumber TEXT NOT NULL,
    artist TEXT NOT NULL,
    performer TEXT NOT NULL,
    copyright TEXT NOT NULL,
    license TEXT NOT NULL,
    organization TEXT NOT NULL,
    description TEXT NOT NULL,
    genre TEXT NOT NULL,
    date TEXT NOT NULL,
    location TEXT NOT NULL,
    contact TEXT NOT NULL,
    isrc TEXT NOT NULL,
    outcast TEXT NOT NULL,
    FOREIGN KEY (file_id) REFERENCES files(id)
);

