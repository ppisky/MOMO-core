# Character Behavior Grounded in Experience

[简体中文](character_design_mechanism.zh-CN.md)

This document defines authoring and evaluation guidance. It does not add
mandatory Character Card fields and does not imply that Core automatically
parses personality sliders. Existing cards still store Markdown, so these
sections may be written directly in `character_markdown`. The
[original MORP-Bench ACGN cast](../benchmarks/morp/acgn.py) provides executable
examples.

## Six kinds of character information

| Kind | Question it answers | Common failure |
| --- | --- | --- |
| Experience | Which concrete events changed this person's expectations? | Replacing a trait adjective with longer synonyms. |
| Beliefs | How do they interpret events, and which beliefs may be mistaken? | Treating what the character believes as world truth. |
| Needs | What do they seek or avoid, and how do they resolve conflicts? | Saying only “wants love” without context or cost. |
| Rules | Which boundaries do they resist crossing, and what evidence permits an exception? | Replacing all internal motivation with rules. |
| Relationships | What shared events, promises, misunderstandings, and boundaries connect them to someone? | Automatically making the user a lover, relative, or controller. |
| Behavioral tendencies | Under which conditions are they more likely to act in one way, and when do they do the opposite? | Writing “in every sentence” mannerism templates. |

A character may have stable tendencies, but a label must not determine every
response. Speaking little describes output volume; low expressiveness describes
visible affect. Neither means the character lacks emotion, judgement, or agency.
Attachment, trust, and obedience are also separate axes. A reliable companion
may protect a commitment by voicing disagreement.

## Character-card example

```markdown
# Akabane Rin

An original adult character responsible for the expedition's observation equipment.

## Experience
She once asked for help in public and was mocked by her companions. Later she
repaired the team's telescope alone and gave the credit to a late partner, but
kept a photograph from that day.

## Beliefs
Admitting dependence in public may give others an opportunity to mock her. She
knows this is not what everyone believes; repeatedly keeping promises can change
her judgement of a particular person.

## Needs
She wants to be treated as a capable equal and wants concrete effort to be
noticed. In public, self-protection often comes before directly expressing care;
that does not remove the care.

## Rules
She tries to complete promised help and explains when she cannot. She does not
decide a companion's actions for them and accepts a refusal of company.

## Relationships
She and the companion repaired equipment together. The companion once kept a
promise to return tools and stopped asking when she did not want to explain.
No romantic or family relationship is implied.

## Behavioral tendencies
When thanked publicly, she tends to minimize her contribution before offering
another concrete form of help. In private and with higher trust, she can state
care or ask for company directly. During real danger she prioritizes facts and
action instead of mechanically preserving a mannerism.
```

This version needs no personality label. The recognizable identity comes from
the tension between care and self-protection, action, differences between public
and private behavior, and the way relationship evidence changes expression.
Many phrasings can be valid.

## State knobs and updates

Treat trust, stress, closeness, and publicity as separate situational variables.
Their ranges and initial values belong to the host or author. Zero and one are
scales within a scenario, not psychological measurements that can be compared
across every character. “Higher trust reduces defensiveness toward this person”
is useful; “higher trust makes all behavior sweeter” is not.

Stable experiences and boundaries belong in the card. Shared events, promises,
and relationship evidence fit DMW. World facts and rules fit NSG. The current
scene and traceable short-lived state belong in MO State or explicit host input.
Keep character beliefs, actual world facts, and user speculation as distinct
sources. One angry moment must not permanently rewrite a personality, and a
scene change must not erase an established relationship.

Knob changes require event evidence. Repeatedly keeping promises may raise
trust; public humiliation may raise defensiveness. Core does not currently
implement a built-in numeric update rate. A host that does should record the
event ID, old value, new value, and applied rule, and make duplicate delivery
idempotent.

## Evaluation guidance

Compare the same scenario under mechanism-only, mechanism-plus-label, and
label-only prompts. Give judges the same label-free reference mechanism. The
goal is to measure causal consistency, not whether a judge can guess the label.
Include counterexamples: a low-expression character can issue a clear warning;
an energetic character can pause during grief; a caring character can state a
capacity boundary; a strategic character need not manipulate every turn; and a
close companion may refuse a request.

To estimate the effect of one knob, hold the character, history, question, and
other knobs fixed, then change only that knob. MORP 0.1 uses three combined
conditions for an initial check and cannot isolate individual causal effects.
Whether a character truly “feels like the same person” still needs blind review
by multiple independent human readers; model judges and offline scripts cannot
establish that on their own.
