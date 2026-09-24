//! A tiny LV2 Für Elise source. The bundled SMF file is the single source of
//! note pitches and timing for both the MIDI and square-wave audio outputs.

use lv2::prelude::*;
use midly::{MetaMessage, MidiMessage as SmfMidiMessage, Smf, Timing, TrackEventKind};

const FUR_ELISE: &[u8] = include_bytes!("../furelise.lv2/FurElise.MID");
const ATTACK_SECONDS: f64 = 0.055;
const DECAY_SECONDS: f64 = 0.025;
const SUSTAIN_LEVEL: f64 = 0.55;
const RELEASE_SECONDS: f64 = 0.045;

struct Note {
    pitch: u8,
    ticks: u32,
}

/// Decode a standard MIDI file with `midly`; the synth currently selects its
/// first track because it has a single square-wave voice.
fn parse_smf(bytes: &[u8]) -> Option<(u16, u32, Vec<Note>)> {
    let smf = Smf::parse(bytes).ok()?;
    let Timing::Metrical(division) = smf.header.timing else {
        return None;
    };
    let mut tempo_micros = 500_000;
    let mut elapsed_ticks = 0_u32;
    let mut active_note = None;
    let mut notes = Vec::new();

    for event in smf.tracks.first()? {
        elapsed_ticks = elapsed_ticks.checked_add(event.delta.as_int())?;
        match event.kind {
            TrackEventKind::Meta(MetaMessage::Tempo(tempo)) => tempo_micros = tempo.as_int(),
            TrackEventKind::Midi {
                message: SmfMidiMessage::NoteOn { key, vel },
                ..
            } if vel.as_int() != 0 => active_note = Some((key.as_int(), elapsed_ticks)),
            TrackEventKind::Midi {
                message: SmfMidiMessage::NoteOff { key, .. },
                ..
            } => {
                if let Some((pitch, start)) = active_note.take() {
                    if pitch == key.as_int() {
                        notes.push(Note {
                            pitch,
                            ticks: (elapsed_ticks - start).max(1),
                        });
                    }
                }
            }
            TrackEventKind::Midi {
                message: SmfMidiMessage::NoteOn { key, vel },
                ..
            } if vel.as_int() == 0 => {
                if let Some((pitch, start)) = active_note.take() {
                    if pitch == key.as_int() {
                        notes.push(Note {
                            pitch,
                            ticks: (elapsed_ticks - start).max(1),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    (!notes.is_empty()).then_some((division.as_int(), tempo_micros, notes))
}

#[derive(URIDCollection)]
struct Urids {
    atom: AtomURIDCollection,
    midi: MidiURIDCollection,
    unit: UnitURIDCollection,
}

#[derive(PortCollection)]
struct Ports {
    audio_out: OutputPort<Audio>,
    midi_out: OutputPort<AtomPort>,
}

#[derive(FeatureCollection)]
struct Features<'a> {
    map: LV2Map<'a>,
}

#[uri("https://joebutton.co.uk/lv2/furelise")]
struct FurElise {
    sample_rate: f64,
    frames_per_tick: f64,
    notes: Vec<Note>,
    urids: Urids,
    note_index: usize,
    note_start: u64,
    absolute_frame: u64,
    phase: f64,
    started: bool,
}

impl FurElise {
    fn note_frames(&self) -> u64 {
        (f64::from(self.notes[self.note_index].ticks) * self.frames_per_tick)
            .round()
            .max(1.0) as u64
    }

    fn write_note(
        sequence: &mut lv2::lv2_atom::sequence::SequenceWriter<'_, '_>,
        midi: URID<MidiEvent>,
        frame: u32,
        status: u8,
        note: u8,
    ) {
        if let Some(mut event) = sequence.init(TimeStamp::Frames(i64::from(frame)), midi, ()) {
            let _ = event.write_raw(&[status, note, 100], true);
        }
    }
}

impl Plugin for FurElise {
    type Ports = Ports;
    type InitFeatures = Features<'static>;
    type AudioFeatures = ();

    fn new(info: &PluginInfo, features: &mut Features<'static>) -> Option<Self> {
        let (division, tempo_micros, notes) = parse_smf(FUR_ELISE)?;
        Some(Self {
            sample_rate: info.sample_rate(),
            frames_per_tick: info.sample_rate() * f64::from(tempo_micros)
                / 1_000_000.0
                / f64::from(division),
            notes,
            urids: features.map.populate_collection()?,
            note_index: 0,
            note_start: 0,
            absolute_frame: 0,
            phase: 0.0,
            started: false,
        })
    }

    fn run(&mut self, ports: &mut Ports, _: &mut (), _: u32) {
        let Some(mut midi) = ports.midi_out.init(
            self.urids.atom.sequence,
            TimeStampURID::Frames(self.urids.unit.frame),
        ) else {
            return;
        };
        if !self.started {
            Self::write_note(&mut midi, self.urids.midi.raw, 0, 0x90, self.notes[0].pitch);
            self.started = true;
        }

        for (frame, sample) in ports.audio_out.iter_mut().enumerate() {
            if self.absolute_frame - self.note_start >= self.note_frames() {
                let old_note = self.notes[self.note_index].pitch;
                Self::write_note(&mut midi, self.urids.midi.raw, frame as u32, 0x80, old_note);
                self.note_index = (self.note_index + 1) % self.notes.len();
                self.note_start = self.absolute_frame;
                self.phase = 0.0;
                Self::write_note(
                    &mut midi,
                    self.urids.midi.raw,
                    frame as u32,
                    0x90,
                    self.notes[self.note_index].pitch,
                );
            }

            let duration = self.note_frames() as f64;
            let position = (self.absolute_frame - self.note_start) as f64;
            let attack = (ATTACK_SECONDS * self.sample_rate)
                .min(duration * 0.4)
                .max(1.0);
            let decay = (DECAY_SECONDS * self.sample_rate)
                .min((duration - attack) * 0.4)
                .max(1.0);
            let release = (RELEASE_SECONDS * self.sample_rate)
                .min(duration * 0.3)
                .max(1.0);
            let envelope = if position < attack {
                position / attack
            } else if position < attack + decay {
                1.0 - (1.0 - SUSTAIN_LEVEL) * ((position - attack) / decay)
            } else if position > duration - release {
                SUSTAIN_LEVEL * ((duration - position) / release).max(0.0)
            } else {
                SUSTAIN_LEVEL
            };
            let note = self.notes[self.note_index].pitch;
            let hz = 440.0 * 2.0_f64.powf((f64::from(note) - 69.0) / 12.0);
            self.phase = (self.phase + hz / self.sample_rate) % 1.0;
            *sample = (if self.phase < 0.5 { 1.0 } else { -1.0 }) * envelope as f32 * 0.18;
            self.absolute_frame += 1;
        }
    }
}

lv2_descriptors!(FurElise);
