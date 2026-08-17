# Third-party license notes

MOMO source code is distributed under Apache License 2.0. Third-party
dependencies and bundled assets remain under their respective licenses.

The dependency snapshot in `Cargo.lock` and
`apps/momo-app/pubspec.lock` was reviewed on 2026-07-30:

- no direct dependency requires the MOMO source code to be distributed under
  GPL, AGPL, or LGPL;
- Rust registry dependencies use permissive licenses including Apache-2.0,
  MIT, BSD, ISC, Zlib, Unicode-3.0, MPL-2.0, and compatible combinations;
- `r-efi` offers `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; MOMO uses it under
  the MIT or Apache-2.0 option, not LGPL;
- `allo-isolate` omits the SPDX field in its Cargo metadata, but its packaged
  `LICENSE` file is Apache-2.0;
- Flutter and Dart packages in the locked package graph use BSD-style, MIT, or
  Apache-2.0 licenses. Flutter SDK subpackages inherit Flutter's BSD license;
- the vendored `cargokit` integration retains its upstream MIT and Apache-2.0
  license text at `apps/momo-app/rust_builder/cargokit/LICENSE`;
- the bundled Sarasa Gothic font files remain under SIL Open Font License 1.1.
  Its license is stored beside the font files.
- `docs/spec_v1.md` and `docs/spec_v2.md` are unmodified reference snapshots
  from `malfoyslastname/character-card-spec-v2` at commit
  `8083fb388615ccbce768e97cbbd49d2b3214632c`. That upstream repository did not
  declare a license when verified on 2026-08-17; these documents remain
  upstream material and are not relicensed under MOMO's Apache-2.0 license.
- `docs/spec_v3.md` is an unmodified reference snapshot from
  `kwaroran/character-card-spec-v3` at commit
  `f3a86af019fbd99f788f7a1155f399655b34ab35`. It is distributed under the MIT
  License; the upstream notice is retained in
  `docs/character-card-spec-v3.LICENSE`.

This is an engineering audit of the current lockfiles, not legal advice.
Dependency updates must repeat the audit and retain all required notices.
