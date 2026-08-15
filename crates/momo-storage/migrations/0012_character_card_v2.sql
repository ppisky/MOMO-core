ALTER TABLE character_cards ADD COLUMN author_name TEXT NOT NULL DEFAULT '';
ALTER TABLE character_cards ADD COLUMN author_url TEXT;
ALTER TABLE character_cards ADD COLUMN opening_markdown TEXT;

UPDATE character_cards
SET author_name = author_display_name
WHERE author_name = '';
