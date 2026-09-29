#![no_std]

use core::ffi::{c_char, c_void, CStr};
use core::mem::size_of;
use core::panic::PanicInfo;
use core::ptr;

const MIDI: &[u8] = include_bytes!("../furelise.lv2/FurElise.MID");
const URI: &[u8] = b"https://joebutton.co.uk/lv2/furelise\0";
const URID_MAP: &[u8] = b"http://lv2plug.in/ns/ext/urid#map\0";
const ATOM_SEQUENCE: &[u8] = b"http://lv2plug.in/ns/ext/atom#Sequence\0";
const MIDI_EVENT: &[u8] = b"http://lv2plug.in/ns/ext/midi#MidiEvent\0";
const MAX_NOTES: usize = 64;

#[repr(C)]
struct Feature {
    uri: *const c_char,
    data: *mut c_void,
}

#[repr(C)]
struct UridMap {
    handle: *mut c_void,
    map: unsafe extern "C" fn(*mut c_void, *const c_char) -> u32,
}

#[repr(C)]
struct Descriptor {
    uri: *const c_char,
    instantiate: unsafe extern "C" fn(
        *const Descriptor,
        f64,
        *const c_char,
        *const *const Feature,
    ) -> *mut c_void,
    connect_port: unsafe extern "C" fn(*mut c_void, u32, *mut c_void),
    activate: Option<unsafe extern "C" fn(*mut c_void)>,
    run: unsafe extern "C" fn(*mut c_void, u32),
    deactivate: Option<unsafe extern "C" fn(*mut c_void)>,
    cleanup: unsafe extern "C" fn(*mut c_void),
    extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

// The descriptor is immutable after construction.
unsafe impl Sync for Descriptor {}

#[repr(C)]
struct Atom {
    size: u32,
    kind: u32,
}

#[repr(C)]
struct Sequence {
    atom: Atom,
    unit: u32,
    pad: u32,
}

#[derive(Clone, Copy)]
struct Note {
    pitch: u8,
    ticks: u32,
}

const EMPTY_NOTE: Note = Note { pitch: 0, ticks: 0 };

#[derive(Clone, Copy)]
struct PreparedNote {
    pitch: u8,
    frames: u64,
    phase_step: f32,
    attack_end: f32,
    decay_end: f32,
    release_start: f32,
    attack_gain: f32,
    decay_gain: f32,
    release_gain: f32,
}

struct State {
    audio: *mut f32,
    midi: *mut Sequence,
    phase: f32,
    frame: u64,
    note_start: u64,
    sequence_urid: u32,
    midi_urid: u32,
    notes: [PreparedNote; MAX_NOTES],
    note_count: usize,
    note_index: usize,
    started: bool,
}

unsafe extern "C" {
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn pow(base: f64, exponent: f64) -> f64;
    fn floor(value: f64) -> f64;
}

fn be16(bytes: &[u8]) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(..2)?.try_into().ok()?))
}

fn be32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?))
}

fn vlq(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    for _ in 0..4 {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value = value.checked_shl(7)? | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn parse_midi(bytes: &[u8]) -> Option<(u16, u32, [Note; MAX_NOTES], usize)> {
    if bytes.get(..4)? != b"MThd" {
        return None;
    }
    let header_size = usize::try_from(be32(bytes.get(4..)?)?).ok()?;
    let division = be16(bytes.get(12..)?)?;
    if division == 0 || division & 0x8000 != 0 {
        return None;
    }
    let track_start = 8usize.checked_add(header_size)?;
    if bytes.get(track_start..track_start + 4)? != b"MTrk" {
        return None;
    }
    let track_size = usize::try_from(be32(bytes.get(track_start + 4..)?)?).ok()?;
    let end = track_start.checked_add(8)?.checked_add(track_size)?;
    let track = bytes.get(track_start + 8..end)?;
    let mut notes = [EMPTY_NOTE; MAX_NOTES];
    let (mut cursor, mut elapsed, mut tempo, mut status, mut count) =
        (0usize, 0u32, 500_000u32, 0u8, 0usize);
    let mut active: Option<(u8, u32)> = None;
    while cursor < track.len() {
        elapsed = elapsed.checked_add(vlq(track, &mut cursor)?)?;
        let next = *track.get(cursor)?;
        if next & 0x80 != 0 {
            status = next;
            cursor += 1;
        }
        if status == 0xff {
            let kind = *track.get(cursor)?;
            cursor += 1;
            let size = usize::try_from(vlq(track, &mut cursor)?).ok()?;
            let data = track.get(cursor..cursor.checked_add(size)?)?;
            cursor += size;
            if kind == 0x51 && size == 3 {
                tempo = u32::from_be_bytes([
                    0,
                    unsafe { *data.get_unchecked(0) },
                    unsafe { *data.get_unchecked(1) },
                    unsafe { *data.get_unchecked(2) },
                ]);
            }
            if kind == 0x2f {
                break;
            }
        } else if status == 0xf0 || status == 0xf7 {
            let size = usize::try_from(vlq(track, &mut cursor)?).ok()?;
            cursor = cursor.checked_add(size)?;
            track.get(..cursor)?;
        } else {
            if status & 0x80 == 0 {
                return None;
            }
            let pitch = *track.get(cursor)?;
            cursor += 1;
            let kind = status & 0xf0;
            let velocity = if kind == 0xc0 || kind == 0xd0 {
                0
            } else {
                let value = *track.get(cursor)?;
                cursor += 1;
                value
            };
            if kind == 0x90 && velocity != 0 {
                active = Some((pitch, elapsed));
            } else if (kind == 0x80 || (kind == 0x90 && velocity == 0))
                && matches!(active, Some((note, _)) if note == pitch)
            {
                let (_, start) = active.take()?;
                if count == MAX_NOTES {
                    return None;
                }
                *unsafe { notes.get_unchecked_mut(count) } = Note {
                    pitch,
                    ticks: elapsed.checked_sub(start)?.max(1),
                };
                count += 1;
            }
        }
    }
    if count == 0 {
        None
    } else {
        Some((division, tempo, notes, count))
    }
}

fn frequency(pitch: u8) -> f64 {
    unsafe { 440.0 * pow(2.0, (f64::from(pitch) - 69.0) / 12.0) }
}

fn prepare_note(note: Note, sample_rate: f64, frames_per_tick: f64) -> PreparedNote {
    let frames = unsafe { floor(f64::from(note.ticks) * frames_per_tick + 0.5) }.max(1.0) as u64;
    let duration = frames as f32;
    let rate = sample_rate as f32;
    let attack = (0.055 * rate).min(duration * 0.4).max(1.0);
    let decay = (0.025 * rate).min((duration - attack) * 0.4).max(1.0);
    let release = (0.045 * rate).min(duration * 0.3).max(1.0);
    PreparedNote {
        pitch: note.pitch,
        frames,
        phase_step: (frequency(note.pitch) / sample_rate) as f32,
        attack_end: attack,
        decay_end: attack + decay,
        release_start: duration - release,
        attack_gain: 1.0 / attack,
        decay_gain: 0.45 / decay,
        release_gain: 0.55 / release,
    }
}

unsafe extern "C" fn instantiate(
    _: *const Descriptor,
    sample_rate: f64,
    _: *const c_char,
    features: *const *const Feature,
) -> *mut c_void {
    if features.is_null() || sample_rate <= 0.0 {
        return ptr::null_mut();
    }
    let mut map: *const UridMap = ptr::null();
    for i in 0..64 {
        let feature = unsafe { *features.add(i) };
        if feature.is_null() {
            break;
        }
        if unsafe { CStr::from_ptr((*feature).uri).to_bytes_with_nul() } == URID_MAP {
            map = unsafe { (*feature).data.cast() };
            break;
        }
    }
    if map.is_null() {
        return ptr::null_mut();
    }
    let Some((division, tempo, notes, note_count)) = parse_midi(MIDI) else {
        return ptr::null_mut();
    };
    let sequence_urid = unsafe { ((*map).map)((*map).handle, ATOM_SEQUENCE.as_ptr().cast()) };
    let midi_urid = unsafe { ((*map).map)((*map).handle, MIDI_EVENT.as_ptr().cast()) };
    if sequence_urid == 0 || midi_urid == 0 {
        return ptr::null_mut();
    }
    let state = unsafe { calloc(1, size_of::<State>()).cast::<State>() };
    if state.is_null() {
        return ptr::null_mut();
    }
    let frames_per_tick = sample_rate * f64::from(tempo) / 1_000_000.0 / f64::from(division);
    unsafe {
        (*state).sequence_urid = sequence_urid;
        (*state).midi_urid = midi_urid;
        for (index, note) in notes.iter().copied().take(note_count).enumerate() {
            (*state).notes[index] = prepare_note(note, sample_rate, frames_per_tick);
        }
        (*state).note_count = note_count;
    }
    state.cast()
}

unsafe extern "C" fn connect_port(handle: *mut c_void, port: u32, data: *mut c_void) {
    let state = unsafe { &mut *handle.cast::<State>() };
    match port {
        0 => state.audio = data.cast(),
        1 => state.midi = data.cast(),
        _ => {}
    }
}

unsafe extern "C" fn activate(handle: *mut c_void) {
    let state = unsafe { &mut *handle.cast::<State>() };
    state.frame = 0;
    state.note_start = 0;
    state.note_index = 0;
    state.phase = 0.0;
    state.started = false;
}

unsafe fn write_note(state: &mut State, capacity: u32, frame: u32, status: u8, pitch: u8) {
    let sequence = unsafe { &mut *state.midi };
    if sequence
        .atom
        .size
        .checked_add(24)
        .is_none_or(|size| size > capacity)
    {
        return;
    }
    let used = (sequence.atom.size - 8) as usize;
    let event = unsafe { state.midi.cast::<u8>().add(size_of::<Sequence>() + used) };
    unsafe {
        ptr::write_bytes(event, 0, 24);
        ptr::write_unaligned(event.cast::<i64>(), i64::from(frame));
        ptr::write_unaligned(event.add(8).cast::<u32>(), 3);
        ptr::write_unaligned(event.add(12).cast::<u32>(), state.midi_urid);
        *event.add(16) = status;
        *event.add(17) = pitch;
        *event.add(18) = 100;
    }
    sequence.atom.size += 24;
}

unsafe extern "C" fn run(handle: *mut c_void, sample_count: u32) {
    let state = unsafe { &mut *handle.cast::<State>() };
    if state.audio.is_null() || state.midi.is_null() {
        return;
    }
    let sequence = unsafe { &mut *state.midi };
    let capacity = sequence.atom.size;
    sequence.atom.kind = state.sequence_urid;
    sequence.atom.size = 8;
    sequence.unit = 0;
    sequence.pad = 0;
    if !state.started {
        unsafe {
            write_note(state, capacity, 0, 0x90, state.notes.get_unchecked(0).pitch);
        }
        state.started = true;
    }
    for i in 0..sample_count {
        let note = unsafe { *state.notes.get_unchecked(state.note_index) };
        if state.frame - state.note_start >= note.frames {
            unsafe {
                write_note(state, capacity, i, 0x80, note.pitch);
            }
            state.note_index += 1;
            if state.note_index == state.note_count {
                state.note_index = 0;
            }
            state.note_start = state.frame;
            state.phase = 0.0;
            unsafe {
                write_note(
                    state,
                    capacity,
                    i,
                    0x90,
                    state.notes.get_unchecked(state.note_index).pitch,
                );
            }
        }
        let note = unsafe { *state.notes.get_unchecked(state.note_index) };
        let position = (state.frame - state.note_start) as f32;
        let envelope = if position < note.attack_end {
            position * note.attack_gain
        } else if position < note.decay_end {
            1.0 - (position - note.attack_end) * note.decay_gain
        } else if position > note.release_start {
            ((note.frames as f32 - position) * note.release_gain).max(0.0)
        } else {
            0.55
        };
        state.phase += note.phase_step;
        if state.phase >= 1.0 {
            state.phase -= 1.0;
        }
        unsafe {
            *state.audio.add(i as usize) =
                (if state.phase < 0.5 { 1.0 } else { -1.0 }) * envelope * 0.18;
        }
        state.frame += 1;
    }
}

unsafe extern "C" fn cleanup(handle: *mut c_void) {
    unsafe {
        free(handle);
    }
}

static DESCRIPTOR: Descriptor = Descriptor {
    uri: URI.as_ptr().cast(),
    instantiate,
    connect_port,
    activate: Some(activate),
    run,
    deactivate: None,
    cleanup,
    extension_data: None,
};

#[no_mangle]
pub extern "C" fn lv2_descriptor(index: u32) -> *const c_void {
    if index == 0 {
        (&DESCRIPTOR as *const Descriptor).cast()
    } else {
        ptr::null()
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
