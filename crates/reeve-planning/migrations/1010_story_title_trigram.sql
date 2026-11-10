-- Report from anywhere: similar-title lookup for the duplicate check (GET /stories/similar). pg_trgm ships with
-- PostgreSQL 16 and is a trusted extension; the GIN index serves the `%` operator the query filters with.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX stories_title_trgm ON stories USING gin (title gin_trgm_ops);
