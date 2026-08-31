# Identity Scope Draft (withdrawn)

This unpublished draft is retained only so old links explain why it must not be
implemented. It used one word, *Scope*, for persistent data ownership, request
access, conversations, and character catalogues. That conflation produced the
invalid `character_scope_id` design and made a Core instance look like one
person's directory.

Core 1.0 replaces this draft before publication with
[the Space model](space_model_1_0.md):

- a **Space** is a persistent ownership boundary;
- an access scope is a request-time set of authorized Spaces and is not stored
  as one UUID;
- personal and conversation Spaces are independent;
- characters are globally addressed resources with optional
  `owner_space_id`, never a runtime catalogue UUID;
- MOC v3 selects Space modules independently;
- public 1.0 contracts reject `scope_id`, `conversation_scope_id`, and
  `character_scope_id`.

There is no compatibility alias or migration endpoint for this withdrawn wire
shape.
