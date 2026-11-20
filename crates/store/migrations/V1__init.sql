CREATE TABLE connections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    driver TEXT NOT NULL,
    grp TEXT,
    body TEXT NOT NULL
);

CREATE TABLE history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection TEXT NOT NULL,
    ran_at TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE INDEX history_connection_idx ON history (connection, id DESC);

CREATE TABLE dashboards (
    id TEXT PRIMARY KEY,
    connection TEXT NOT NULL,
    body TEXT NOT NULL
);
