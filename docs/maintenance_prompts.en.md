# MOMO compiled product prompts

[简体中文](maintenance_prompts.zh-CN.md)

MOMO Core owns three System Prompts as reviewable Markdown source files:

- `crates/momo-core/src/product_prompts/dmw_distiller.md` for DMW maintenance;
- `crates/momo-core/src/product_prompts/nsg_governor.md` for NSG governance;
- `crates/momo-core/src/product_prompts/roleplay_director.md` for foreground character performance.

`crates/momo-core/src/product_prompts.rs` embeds these files with `include_str!`.
They are compile-time inputs to `momo_core`, not deployment files and not
runtime configuration.

## Change and runtime boundary

Changing a product prompt requires a Core source change, review, rebuild, and
deployment of the new binary. At runtime, Core does not discover or read a
`prompts/` directory and does not depend on the server working directory.

Product prompts:

- are not fields or paths in `momo.toml`;
- cannot be replaced by a native response request;
- are not imported or exported in a MOC config module;
- are not a Space and do not have Space-level ownership;
- cannot be re-read or hot-reloaded while Core is running.

Open source here means that the complete prompt source is tracked and
reviewable in the Core repository. It does not imply a user-editable runtime
file or a separate prompt service.

## Benchmark boundary

The MORP names `basic_context`, `dmw_nsg`, and `all_enabled` are test-plan
labels, not product prompt variants or Cargo features. They exercise the same
compiled prompt revision. Component switches and counterfactual inputs belong
to the test layer, so prompt or build differences cannot become hidden
experimental inputs.

## Responsibilities

The Roleplay Director governs foreground generation. It turns the Character
Card, transcript, DMW, NSG, and MO State evidence into scene-native performance
without writing memory or taking control of the user.

The DMW Distiller may emit only DMW YAML Patch operations for durable events,
relationships, character development, current scene state, and unresolved
threads. The NSG Governor may propose only durable world rules and graph
relationships; automatic changes remain Draft and cannot directly mutate Canon.

All product control instructions and structural keys are English. Narrative
content may use any language and must preserve source spelling and meaning.
Core does not use a language-specific keyword list or a hidden CJK/Latin gate
to decide which facts are valid.

## Core-owned validation

From the MOMO Core repository root, run:

```bash
cargo test -p momo_core product_prompts::tests::compiled_product_prompts_are_complete
```

This verifies the three compile-time prompt sources without invoking mobot or a
model.
