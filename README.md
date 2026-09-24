# Für Elise LV2

A simple LV2 plugin written in Rust, for those occasions when you're testing
something and you want a sound source right now but you can't be bothered to set
up proper audio or MIDI inputs.

It loops the bundled opening of **Für Elise** and outputs it via both MIDI and
audio outs. I was initially thinking of using Daisy Bell but Für Elise seems
considerably less grating.

Build a ready-to-use bundle containing a regular copy of the plugin library:

```sh
make
```

That creates a `furelise.lv2` bundle under `build/`. Theoretically it should
work on Linux / macOS / Windows, but I've only tested it on Linux.
