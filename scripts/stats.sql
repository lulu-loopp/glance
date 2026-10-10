-- The site's own count (see _worker.js): one row for each page shown and
-- each installer taken; the salts that tell one day's visitors apart.
CREATE TABLE IF NOT EXISTS events (
  ts INTEGER NOT NULL,
  day TEXT NOT NULL,
  kind TEXT NOT NULL,
  path TEXT NOT NULL,
  ref TEXT NOT NULL,
  country TEXT NOT NULL,
  visitor TEXT NOT NULL,
  source TEXT NOT NULL,
  lang TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_day ON events (day, kind);
CREATE TABLE IF NOT EXISTS salts (day TEXT PRIMARY KEY, salt TEXT NOT NULL);
