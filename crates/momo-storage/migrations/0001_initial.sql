PRAGMA foreign_keys = ON;

CREATE TABLE character_cards (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    description TEXT NOT NULL,
    language TEXT NOT NULL,
    tags TEXT NOT NULL,
    author_uid TEXT NOT NULL,
    author_display_name TEXT NOT NULL,
    character_markdown TEXT NOT NULL,
    user_markdown TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    character_id TEXT REFERENCES character_cards(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant')),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX messages_conversation_created_idx ON messages(conversation_id, created_at, id);
