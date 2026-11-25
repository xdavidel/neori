This is a checklist of things to release a new neori version.

neori numbers its own versions as `major.minor.patch`, separate from niri's `year.month`
versions. `0.x` versions can still change the Lua API; a minor bump (`0.2.0` → `0.3.0`) adds
features, and a patch bump (`0.2.1`) only fixes bugs. We'll use `0.3.0` as the example new
version.

## Write the release notes

Go through the commits since the last tag (`git log v0.2.0..`) and sum up what changed for users:
new features, fixes, and anything that needs a config change. They go into the tag's message.

While doing this, also check:

- that additions are marked with `<sup>Since: next release</sup>` on the wiki,
- if anything needs updating in `README.md` (the "Neori features" list).

## Bump the version

The version lives in the workspace's `Cargo.toml` and is shared by every crate. With
[cargo-edit](https://github.com/killercup/cargo-edit):

```
cargo set-version 0.3.0
```

Or edit it by hand: `[workspace.package]` and the `version = "…"` of the internal dependencies
(`neori-config`, `neori-ipc`, `neori-config-derive`, `neori`) in the `Cargo.toml` files.

Then update the version in:

- `[package.metadata.generate-rpm]` in `Cargo.toml`,
- the dependency example in `neori-ipc/README.md`,
- the dependency example in `neori-ipc/src/lib.rs`.

Do a full text search for the old version to make sure there are no other places. The Nix
package reads the version from `Cargo.toml`, as `0.3.0-<commit>`, and `neori --version` prints
`0.3.0 (<commit>)`.

## Replace all "Since: next release" mentions

Do a full text search for `next release` in `docs/wiki` and replace each with the new version:
`<sup>Since: 0.3.0</sup>`. Older markers like `Since: 25.08` are niri versions, from before
neori had its own.

## Build and test

```
RUN_SLOW_TESTS=1 cargo test --release --all
```

Also make sure the Nix package builds: `nix build`.

## Commit, tag and push

Commit the version bump, then make an annotated tag, with the release notes as its message:

```
git tag -a v0.3.0
git push origin main v0.3.0
```

The tag starts with a `v`. Use an annotated tag rather than a lightweight one: it records who made
it and when, and works better with tools like `git describe`.

## Publish on GitLab

Create a release from the tag on GitLab, with the release notes.

## Update the systems that use it

Systems that use the flake pick it up with `nix flake update neori`.
