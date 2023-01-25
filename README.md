<p align="center"><img width="200px" alt="pilfer logo" src="https://github.com/eludris/pilfer/blob/main/assets/pilfer.png" /></p>

# Pilfer

A simple TUI for Eludris made in rust.

![An image of pilfer in action](https://github.com/eludris/pilfer/blob/main/assets/pilfer-preview.png)

## Usage

To use pilfer either download a binary from the releases page or building it
locally with

```sh
cargo install pilfer
```

Pilfer is also available on the [AUR](https://aur.archlinux.org/packages/pilfer):

```sh
<your-favourite-aur-helper> -S pilfer
```

You can *also* yoink the precompiled binaries from the [releases page](https://github.com/eludris/pilfer/releases/latest)
of this repository.

Pilfer defaults to using @ooliver1's Eludris instance located at <https://eludris.tooty.xyz/>,
to change that overwrite the `REST_URL` environment variable.

You can also define a default name using the `PILFER_NAME` environment variable,
it has to be > 1 characters < 33 characters.
