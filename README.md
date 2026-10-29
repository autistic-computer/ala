# gpui

Our fork of [GPUI](https://gpui.rs), the GPU UI framework from [Zed](https://github.com/zed-industries/zed), split out of Zed with the utility crates it needs folded in.

| Crate | What it is |
|---|---|
| `zz-gpui` | The framework: elements, layout, windows, text, the scene. Zed's `collections`, `sum_tree`, `scheduler`, `util`, `http_client` and `refineable` live in it as modules. |
| `zz-gpui-macros` | Proc macros, including the `Refineable` derive. |
| `zz-gpui-platform` | The backends, each compiled only for its own target: `macos` (Metal, in `apple`), `linux` (Wayland, X11), `windows`, `web`, `ios` (UIKit), and the `wgpu` renderer that Linux, web and iOS share. |

`third_party/cosmic-text` is a patched cosmic-text used by the wgpu text system. See its `UPSTREAM.md`.

## Use it

```toml
[dependencies]
zz-gpui = { git = "https://github.com/autistic-computer/gpui", branch = "main" }
zz-gpui-platform = { git = "https://github.com/autistic-computer/gpui", branch = "main", features = ["font-kit", "wayland", "x11"] }
```

Needs Rust 1.99. Try `cargo run -p zz-gpui --example hello_world`, and read `crates/zz-gpui/README.md` for the framework itself.

## Where changes come from

[zz](https://github.com/demfabris/zz) carries this repo as a git subtree in `libs/gpui`, edits it in place and sends its commits here with `git subtree push`. Commits made here directly go back into zz with `git subtree pull`. Both work, so open PRs against whichever repo you're in.

## License

Apache-2.0, like upstream gpui. See `LICENSE-APACHE`.
