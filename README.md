<p align="center"><img src="docs/logo.png" alt="avatar aura" width="640"></p>

Desktop tool for Xbox 360 avatars for use with [AvatarEditorRecomp](https://github.com/panedivitasei/AvatarEditorRecomp): import avatar items into the Avatar Editor. Pose and animate your avatar, then export it as a 3D model.

**Export**
- Load your saved avatar and preview it with materials and animations.
- Plays every AE face and body animation. 
- Mouth, eyes and brow expressions, rendered from your own avatar.
- Pose your avatar without an external app.
- Export your avatar to a common format. 

**Import**
- Reads STFS containers and raw `.bin` avatar items with a live preview. 
- Adds the items and the Avatar Award relevant game icon to your Avatar Editor closet.

## Requirements

- Windows 10 or later
- [AvatarEditorRecomp](https://github.com/panedivitasei/AvatarEditorRecomp)

## Building

1. Install Rust and clone AvatarEditorRecomp.
2. `cargo run -p build-assets -- <path to AvatarEditorRecomp>`
3. `cargo build --release`

## Credits

[Project Aura](https://project-archives.etc.cmu.edu/2010/spring/aura/)

[360css](https://github.com/Tarmo1/360css)

