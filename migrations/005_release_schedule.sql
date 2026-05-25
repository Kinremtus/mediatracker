CREATE TABLE release_schedule (
  id SERIAL PRIMARY KEY,
  provider TEXT NOT NULL,
  external_id TEXT NOT NULL,
  episode_number INT NOT NULL,
  air_date TIMESTAMPTZ NOT NULL,
  title TEXT NOT NULL,
  poster_url TEXT,
  fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(provider, external_id, episode_number)
);
CREATE INDEX idx_release_air_date ON release_schedule(air_date);
