# Building

1. Install Rust (stable) and clone AvatarEditorRecomp.
2. cargo run -p build-assets -- <path to AvatarEditorRecomp>
3. cargo build --release

Step 2 fills `crates/avatar-aura/assets/` with the game-derived files the repo leaves out: the bundled clips, the two mannequins (from the DryCleaner build output), the face layer PNGs with `face_index.json`, and `catalog.json`. It bakes the faces from the recomp's asset pack, so it takes a couple of minutes.
Inputs default to the recomp tree; `--pack`, `--closet`, `--mannequins`, `--manifest` and `--out` override them (`--help` lists them).
`cargo run --release -p avatar-aura -- --smoke` checks the result; it prints `smoke: ok`.
