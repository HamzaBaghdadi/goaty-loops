# Roadmap

Full feature parity with a mature commercial DAW is a multi-year effort. This is the
order of work, chosen to make Goaty Loops useful for real projects as early as possible.

## Road to 1.0 (stable)

What a first stable release needs, in order. Each step should leave the app usable.

1. ~~**Audio device settings**~~ (done): OPTIONS > Audio settings picks the driver, output device, sample rate and buffer size and shows the real latency; settings persist; an unplugged device falls back to the default. Still to do: follow Windows default-device changes while running.
2. **Low-latency drivers**: ~~WASAPI exclusive / low-latency shared mode~~ (done, `wasapi.rs`: 3 ms buffers, 6 ms output latency on a Realtek laptop codec); ASIO as an optional build is left (`cpal` `asio` feature; the ASIO SDK is GPLv3 or proprietary-licensed, so check licensing before shipping ASIO builds; building it needs the MSVC toolchain and LLVM).
3. **Reliability**: ~~autosave and crash recovery, a crash reporter~~ (done: autosave every 2 min into `%APPDATA%\Goaty Loops\Autosave`, recovery dialog after a crash or kill, crash reports naming the crashing module, emergency save on a UI panic, atomic saves, "save changes?" on close/new/open). Left: plugin crash isolation for effects as well as the scan, and a fuzzed FLP/`.goaty` loader that never panics.
4. **Mixing & mastering basics**: built-in parametric EQ with analyser, limiter/maximizer, multiband compressor, sidechain routing UI, LUFS/peak metering on the master.
5. **Editing workflow gaps**: automation lanes, audio slicing, pattern length, loop/punch markers, multi-take recording.
6. **CLAP hosting** next to VST3.
7. **Packaging**: Windows installer with file associations (`.goaty`, `.flp`) and the app icon, signed builds, versioned releases with changelogs; then macOS/Linux.
8. **Documentation**: a user manual and keyboard-shortcut reference.

## Milestone 1 - Foundation (done)
- [x] FLP parser (event stream, channels, patterns, notes, playlist, mixer)
- [x] Real-time engine: sequencer, voices, mixer buses, routing, meters
- [x] Sampler, Goaty Synth, Goaty Drum; delay, reverb, filter, distortion, compressor, EQ
- [x] Channel rack, step sequencer, piano roll, playlist, mixer, browser
- [x] Undo/redo, `.goaty` save/load (`.bcproj` from before the rename still opens), WAV export, experimental FLP export

## Milestone 2 - Plugin hosting (mostly done)
- [x] VST3 hosting (`vst3` crate): instruments in the channel rack, effects in mixer slots, sample-accurate notes, tempo/transport info
- [x] Crash-safe plugin scanner (one child process per bundle) with a cached database (`%APPDATA%\Goaty Loops\plugins.json`)
- [x] Plugin editor windows (Win32), with editor edits forwarded to the audio thread
- [x] Plugin state saved in `.goaty` and restored on load
- [x] FLP import maps FL "Fruity Wrapper" plugins to installed VST3s and restores their state (incl. bypassed slots)
- [x] Mixer tracks processed in parallel across CPU cores
- [ ] CLAP hosting (`clack`)
- [x] Plugin delay compensation for mixer effects: paths through the routing tree are aligned at every destination (direct audio and sends delayed as needed)
- [x] Delay compensation for plugin instruments (built-in voices and faster instruments are delayed to the slowest); total compensation shown in OPTIONS
- [x] VST3 parameter automation, MIDI learn, and MIDI CC / pitch bend to plugin instruments (via IMidiMapping: mod wheel, sustain, expression...)
- [ ] Plugin editors on macOS/Linux
- [x] Plugins load one per frame after a project opens, with progress in the status bar (VST3 needs the UI thread, so no background thread); failures go to the import report

## Milestone 3 - FLP fidelity (in progress)
- [x] Automation clips: points and curve tension, playlist placement, values held between clips; targets: channel volume/pan, mixer volume/pan, slot mix and on/off, FL effect parameters, VST3 parameters, tempo, main volume
- [x] Channel pitch (knob, range, import, automation)
- [x] Automation clip editor (points, target picker), creating and painting automation clips
- [x] Mixer routing to several inserts (FL sends, in any direction) with send levels, imported from FLP; sends shown and editable in the mixer side panel; processing in dependency order with per-route delay compensation
- [x] FL's volume law measured in FL (gain = (e^(2.4x)-1)/(e^2.4-1), x = raw/12800) for mixer faders and sends; channel volume uses the same law re-centred so the default 78% = unity (verified by FL renders: 52% = -6.9 dB)
- [x] Sampler preamp (event 74, gain = 1 + v/16), audio clip gain, and trimming plugin-delay silence from exports like FL; a full render of a real song now matches FL's own render within 0.7 dB overall and 0.0 dB on solo sections
- [x] Routing edits (outputs, sends, send levels) written back to the FLP; untouched inserts stay byte-identical
- [x] Sidechain: FL "sidechain to this track" routes (send level 0) and the Fruity Limiter compressor's SIDECHAIN input (field 41), decoded from test files made in FL; the compressor follows the sidechain signal
- [ ] Automation of FL synth parameters (Sytrus, Harmor, FLEX...), channel filter; curve tension editing
- [x] Sampler volume envelope (ADSR + hold) and cut groups, imported from FL
- [x] FL 3x Osc decoded into Goaty Synth (waveforms, coarse/fine tuning, oscillator mix, channel volume envelope); 184 instances across the test projects
- [x] Stand-ins for unavailable instruments (FLEX, Sytrus, missing VSTs) are voiced from the preset/channel name: 808, bass, piano, guitar/pluck, bells, pads/strings, leads
- [x] Sampler reverse (FX flags event), time stretch (TIME length, pitch, resample vs. pitch-preserving modes) and audio clip fade in/out: decoded from FL test files, played back, and written back byte-identical to FL
- [ ] More sampler settings: sample start/trim, fade stereo, swap stereo, layering; channel filter envelope
- [x] FL native effects, decoded with their settings: Parametric EQ 2, Limiter, Reeverb 2, Compressor, Delay 2/3, Stereo Enhancer, Stereo Shaper, Balance, PanOMatic, Soft Clipper, Fast Dist, Blood Overdrive, Fruity Filter, Chorus, Soundgoodizer; slot dry/wet mix (75% of native effect instances across 91 test projects; Maximus and Multiband Compressor approximated)
- [x] Fruity Fast LP (with cutoff/resonance automation, also added for Fruity Filter); Transient Processor approximated (attack/release/gain)
- [ ] More FL effects: Maximus (real decode), WaveShaper, Love Philter, Delay Bank, Gross Beat, Effector, Convolver, LuxeVerb, Pitcher, EQUO
- [x] Time markers: imported (incl. pre-12.9 and legacy copies), shown in the ruler, added with Alt+T or right-click, renamed/deleted, and written back to the FLP
- [x] Time-signature markers: imported with their signature, the playlist grid and bar numbers follow them, add from the ruler menu, saved back
- [ ] Pattern length, other marker actions (loop, punch...), playlist track heights
- [x] Round-trip-safe FLP writer: patches the original file (tempo, notes, new patterns, pattern names, playlist clips, channel volume/pan/pitch/mute, mixer volume/pan, slot on/off/mix); unmodified saves are byte-identical (89/89 test projects)
- [x] Channel -> mixer insert routing for FL 20+ (event 104; older files use event 22): imported, and edits written back
- [x] FLP writer: sample channels added in Goaty Loops (copy of an FL sampler or audio clip channel from the same file, with the new name, sample, levels, routing, cut groups), including their notes and audio clips
- [ ] FLP writer: new synth/plugin/automation channels, mixer effect settings, automation edits

## Milestone 4 - Recording & editing (in progress)
- [x] MIDI keyboard input (`midir`): low-latency live playing, recording into patterns, device picker
- [x] MIDI learn (right-click mixer/channel knobs or tempo > Link to MIDI controller), links saved with the project, live pitch bend
- [x] MIDI learn for VST3 plugin parameters via TOOLS > Last tweaked (editor shows the change); FL effect parameters and main volume via live parameter overrides
- [x] Recording notes in song mode: into the pattern clip under the playhead, or a new clip that grows with the take
- [ ] MIDI learn from the built-in effect windows
- [x] Audio input recording (record button > right-click > Audio): takes saved as WAV in Documents/Goaty Loops/Recorded and placed on a free playlist track
- [x] Input device picker, input monitoring (lock-free ring to the output), latency compensation; MIDI/audio device choices persist in %APPDATA%/Goaty Loops/settings.json
- [ ] Automatic latency measurement, multi-take recording (loop takes)
- [x] Multi-select (Ctrl+drag box, Ctrl/Shift+click), group move, copy/cut/paste, duplicate (Ctrl+B), delete, arrow-key nudge/transpose in piano roll and playlist
- [x] Piano roll tools: quantize (starts or starts+lengths), chop, glue, legato, velocity scaling (selection or whole channel)
- [x] Playlist clip splitting (Alt+click), saved to FLP with correct offsets
- [x] FL clip offsets decoded per type (pattern = ticks, audio = milliseconds, automation = beats); trimmed audio clips now start at the right point
- [x] Left-edge clip trimming (keeps the audio/pattern in place)
- [ ] Audio slicing to channels (Slicex-style)
- [ ] Automation lanes and parameter linking
- [x] Time stretching for samples and audio clips (resample, or pitch-preserving grain stretch)
- [ ] Higher-quality stretch algorithms (transient-aware, formant-preserving)

## Milestone 5 - Instruments and effects
- [ ] Wavetable and FM synth, slicer, multi-sample drum rack
- [ ] Limiter, parametric EQ with analyser, chorus/flanger/phaser, sidechain
- [ ] Preset browser and plugin database

## Milestone 6 - Polish
- [ ] Detachable/dockable windows, themes, HiDPI icon set
- [x] Stem export (TOOLS menu): one 32-bit float WAV per mixer insert that carries audio, after its effects and fader, plus the full mix, in one render pass
- [x] FLAC export (24-bit, lossless, pure-Rust encoder)
- [ ] MP3 export (needs an encoder that builds without a C toolchain)
- [x] Real-time audit: the renderer makes no heap allocations in steady playback (mixer jobs iterate in place, automated FL effects decode without copies); regression test `tests/rt_alloc.rs`, `rt_audit` example for real songs
- [x] Dedicated mixer worker pool (`pool.rs`, replaces rayon): workers park between audio blocks instead of spinning, and dispatch never allocates. A 17-insert song went from 6.4 to 1.4 busy cores while playing
- [x] Smart disable: mixer tracks with 3 s of silent input and output skip their effects (tracks with VST3 effects only while stopped, so playback and exports stay sample-exact). Stopped with a song loaded: 1.1 -> 0.2 cores; offline export about 3x faster
- [x] Audio drivers: Windows shared (cpal), WASAPI low-latency shared (`IAudioClient3`) and WASAPI exclusive (any supported rate, buffers from the device minimum), on an MMCSS "Pro Audio" thread; switching reloads plugins at a new sample rate with their settings
- [x] Plugin teardown and project deallocation never run on the audio thread: replaced project versions and effect states are handed to the UI thread to free (some plugins hang if torn down off the main thread)
- [x] Idle UI no longer redraws 20 times a second (MIDI input wakes it); playback redraws are capped at ~60 fps. Empty project idle: 14% -> ~1% of a core
- [ ] Playlist drawing during playback still costs ~0.8 core (it redraws every frame; profile it, likely by caching the waveform shapes)
- [ ] macOS/Linux packaging, installer, crash reporting
