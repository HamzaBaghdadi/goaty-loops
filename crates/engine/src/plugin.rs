//! VST3 plugin hosting: scanning, instantiation, real-time processing and state.
//!
//! The host side implements the COM interfaces a VST3 plugin expects from its
//! host (host application, messages, streams, event lists, parameter changes,
//! component handler). Plugin modules are loaded once and never unloaded.

#![allow(non_snake_case, non_upper_case_globals)]

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper, Interface};
use vst3::Steinberg::Vst::IAttributeList_::AttrID;

/// Description of one plugin class inside a `.vst3` module.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub path: PathBuf,
    /// 32 hex digits.
    pub class_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub sub_categories: String,
    pub instrument: bool,
}

/// Timing information passed to plugins each block.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeInfo {
    pub sample_rate: f64,
    pub tempo: f64,
    pub playing: bool,
    /// Song position in quarter notes.
    pub ppq: f64,
    pub project_samples: i64,
    pub time_sig: (u8, u8),
}

#[derive(Clone, Copy, Debug)]
pub enum NoteEvent {
    On { offset: u32, key: u8, velocity: f32 },
    Off { offset: u32, key: u8 },
}

impl NoteEvent {
    pub fn offset(&self) -> u32 {
        match *self {
            NoteEvent::On { offset, .. } | NoteEvent::Off { offset, .. } => offset,
        }
    }
}

// ---------------------------------------------------------------------------
// Module discovery and loading
// ---------------------------------------------------------------------------

/// Standard VST3 folders for this platform.
pub fn default_vst3_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(cpf) = std::env::var_os("CommonProgramFiles") {
        v.push(PathBuf::from(cpf).join("VST3"));
    }
    if let Some(la) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(la).join("Programs").join("Common").join("VST3"));
    }
    v.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    if let Some(home) = std::env::var_os("HOME") {
        v.push(PathBuf::from(home).join(".vst3"));
    }
    v.push(PathBuf::from("/usr/lib/vst3"));
    v.retain(|p| p.is_dir());
    v
}

/// Finds `.vst3` bundles (directories) and single-file modules under `dirs`.
pub fn find_bundles(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 6 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let is_vst3 = p.extension().map(|x| x.eq_ignore_ascii_case("vst3")).unwrap_or(false);
            if is_vst3 {
                out.push(p);
            } else if p.is_dir() {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        walk(d, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// The binary inside a bundle (or the path itself for single-file modules).
fn module_binary(path: &Path) -> PathBuf {
    if path.is_dir() {
        let stem = path.file_stem().map(|s| s.to_os_string()).unwrap_or_default();
        let arch = if cfg!(target_arch = "aarch64") { "arm64-win" } else { "x86_64-win" };
        let mut bin = path.join("Contents").join(arch).join(&stem);
        bin.set_extension("vst3");
        bin
    } else {
        path.to_path_buf()
    }
}

struct Module {
    _lib: libloading::Library,
    factory: ComPtr<IPluginFactory>,
}

// The factory is only used behind the module cache's mutex or from the UI thread.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

fn modules() -> &'static Mutex<HashMap<PathBuf, Arc<Module>>> {
    static M: OnceLock<Mutex<HashMap<PathBuf, Arc<Module>>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

fn load_module(path: &Path) -> Result<Arc<Module>> {
    let mut map = modules().lock().unwrap();
    if let Some(m) = map.get(path) {
        return Ok(m.clone());
    }
    let bin = module_binary(path);
    unsafe {
        let lib = libloading::Library::new(&bin).with_context(|| format!("loading {}", bin.display()))?;
        if let Ok(init) = lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll") {
            init();
        }
        let get = lib
            .get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory")
            .context("not a VST3 module (no GetPluginFactory)")?;
        let factory = ComPtr::from_raw(get()).ok_or_else(|| anyhow!("plugin returned no factory"))?;
        let m = Arc::new(Module { _lib: lib, factory });
        map.insert(path.to_path_buf(), m.clone());
        Ok(m)
    }
}

fn cstr(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).to_string()
}

fn tuid_hex(t: &TUID) -> String {
    t.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

fn hex_tuid(s: &str) -> Result<TUID> {
    if s.len() != 32 {
        bail!("bad class id {s}");
    }
    let mut t: TUID = [0; 16];
    for (i, c) in t.iter_mut().enumerate() {
        *c = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)? as i8;
    }
    Ok(t)
}

/// Lists the audio processor classes in a module. Loads the module in-process,
/// so callers that need crash safety should run this in a child process.
pub fn scan_module(path: &Path) -> Result<Vec<PluginInfo>> {
    let m = load_module(path)?;
    let mut out = Vec::new();
    unsafe {
        let f2 = m.factory.cast::<IPluginFactory2>();
        let n = m.factory.countClasses();
        for i in 0..n {
            let (cid, category, name, vendor, version, subs) = if let Some(f2) = &f2 {
                let mut info: PClassInfo2 = std::mem::zeroed();
                if f2.getClassInfo2(i, &mut info) != kResultOk {
                    continue;
                }
                (info.cid, cstr(&info.category), cstr(&info.name), cstr(&info.vendor), cstr(&info.version), cstr(&info.subCategories))
            } else {
                let mut info: PClassInfo = std::mem::zeroed();
                if m.factory.getClassInfo(i, &mut info) != kResultOk {
                    continue;
                }
                (info.cid, cstr(&info.category), cstr(&info.name), String::new(), String::new(), String::new())
            };
            if category != "Audio Module Class" {
                continue;
            }
            let vendor = if vendor.is_empty() {
                let mut fi: PFactoryInfo = std::mem::zeroed();
                m.factory.getFactoryInfo(&mut fi);
                cstr(&fi.vendor)
            } else {
                vendor
            };
            out.push(PluginInfo {
                path: path.to_path_buf(),
                class_id: tuid_hex(&cid),
                name,
                vendor,
                version,
                instrument: subs.split('|').any(|s| s.eq_ignore_ascii_case("Instrument")),
                sub_categories: subs,
            });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Host-side COM objects
// ---------------------------------------------------------------------------

fn guid_eq(ptr: *const TUID, iid: &[u8; 16]) -> bool {
    !ptr.is_null() && unsafe { std::slice::from_raw_parts(ptr as *const u8, 16) } == iid
}

fn copy_wstring(src: &str, dst: &mut [TChar]) {
    let mut len = 0;
    for (s, d) in src.encode_utf16().zip(dst.iter_mut()) {
        *d = s as TChar;
        len += 1;
    }
    if len < dst.len() {
        dst[len] = 0;
    }
}

pub(crate) struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        copy_wstring("Goaty Loops", &mut *name);
        kResultOk
    }

    unsafe fn createInstance(&self, cid: *mut TUID, iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        if guid_eq(cid, &IMessage::IID) && guid_eq(iid, &IMessage::IID) {
            let msg = ComWrapper::new(HostMessage { id: Mutex::new(None), attrs: ComWrapper::new(HostAttributes::default()) });
            *obj = msg.to_com_ptr::<IMessage>().unwrap().into_raw() as *mut c_void;
            return kResultOk;
        }
        if guid_eq(cid, &IAttributeList::IID) && guid_eq(iid, &IAttributeList::IID) {
            let a = ComWrapper::new(HostAttributes::default());
            *obj = a.to_com_ptr::<IAttributeList>().unwrap().into_raw() as *mut c_void;
            return kResultOk;
        }
        *obj = std::ptr::null_mut();
        kResultFalse
    }
}

struct HostMessage {
    id: Mutex<Option<CString>>,
    attrs: ComWrapper<HostAttributes>,
}

impl Class for HostMessage {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.lock().unwrap().as_ref().map(|c| c.as_ptr()).unwrap_or(std::ptr::null())
    }
    unsafe fn setMessageID(&self, id: FIDString) {
        *self.id.lock().unwrap() = if id.is_null() { None } else { Some(CStr::from_ptr(id).to_owned()) };
    }
    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        self.attrs.as_com_ref::<IAttributeList>().unwrap().as_ptr()
    }
}

enum AttrValue {
    Int(i64),
    Float(f64),
    Str(Vec<u16>),
    Bin(Vec<u8>),
}

#[derive(Default)]
struct HostAttributes {
    map: Mutex<HashMap<String, AttrValue>>,
}

impl Class for HostAttributes {
    type Interfaces = (IAttributeList,);
}

unsafe fn attr_key(id: AttrID) -> Option<String> {
    (!id.is_null()).then(|| CStr::from_ptr(id).to_string_lossy().to_string())
}

impl IAttributeListTrait for HostAttributes {
    unsafe fn setInt(&self, id: AttrID, value: int64) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        self.map.lock().unwrap().insert(k, AttrValue::Int(value));
        kResultOk
    }
    unsafe fn getInt(&self, id: AttrID, value: *mut int64) -> tresult {
        match attr_key(id).and_then(|k| match self.map.lock().unwrap().get(&k) {
            Some(AttrValue::Int(v)) => Some(*v),
            _ => None,
        }) {
            Some(v) => {
                *value = v;
                kResultOk
            }
            None => kResultFalse,
        }
    }
    unsafe fn setFloat(&self, id: AttrID, value: f64) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        self.map.lock().unwrap().insert(k, AttrValue::Float(value));
        kResultOk
    }
    unsafe fn getFloat(&self, id: AttrID, value: *mut f64) -> tresult {
        match attr_key(id).and_then(|k| match self.map.lock().unwrap().get(&k) {
            Some(AttrValue::Float(v)) => Some(*v),
            _ => None,
        }) {
            Some(v) => {
                *value = v;
                kResultOk
            }
            None => kResultFalse,
        }
    }
    unsafe fn setString(&self, id: AttrID, string: *const TChar) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        let mut v = Vec::new();
        if !string.is_null() {
            let mut p = string;
            while *p != 0 {
                v.push(*p as u16);
                p = p.add(1);
            }
        }
        self.map.lock().unwrap().insert(k, AttrValue::Str(v));
        kResultOk
    }
    unsafe fn getString(&self, id: AttrID, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        let map = self.map.lock().unwrap();
        let Some(AttrValue::Str(s)) = map.get(&k) else { return kResultFalse };
        let cap = (size_in_bytes as usize / 2).saturating_sub(1);
        let n = s.len().min(cap);
        for (i, c) in s.iter().take(n).enumerate() {
            *string.add(i) = *c as TChar;
        }
        *string.add(n) = 0;
        kResultOk
    }
    unsafe fn setBinary(&self, id: AttrID, data: *const c_void, size: uint32) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        let v = if data.is_null() { Vec::new() } else { std::slice::from_raw_parts(data as *const u8, size as usize).to_vec() };
        self.map.lock().unwrap().insert(k, AttrValue::Bin(v));
        kResultOk
    }
    unsafe fn getBinary(&self, id: AttrID, data: *mut *const c_void, size: *mut uint32) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        let map = self.map.lock().unwrap();
        let Some(AttrValue::Bin(v)) = map.get(&k) else { return kResultFalse };
        // The Vec lives as long as the attribute list; the map is not modified while reading.
        *data = v.as_ptr() as *const c_void;
        *size = v.len() as uint32;
        kResultOk
    }
}

/// An in-memory IBStream.
pub(crate) struct MemStream {
    inner: Mutex<(Vec<u8>, usize)>,
}

impl MemStream {
    pub fn new(data: Vec<u8>) -> ComWrapper<MemStream> {
        ComWrapper::new(MemStream { inner: Mutex::new((data, 0)) })
    }
    pub fn data(&self) -> Vec<u8> {
        self.inner.lock().unwrap().0.clone()
    }
}

impl Class for MemStream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for MemStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, num_read: *mut int32) -> tresult {
        let mut g = self.inner.lock().unwrap();
        let (data, pos) = &mut *g;
        let n = (num_bytes.max(0) as usize).min(data.len().saturating_sub(*pos));
        std::ptr::copy_nonoverlapping(data.as_ptr().add(*pos), buffer as *mut u8, n);
        *pos += n;
        if !num_read.is_null() {
            *num_read = n as int32;
        }
        kResultOk
    }
    unsafe fn write(&self, buffer: *mut c_void, num_bytes: int32, num_written: *mut int32) -> tresult {
        let mut g = self.inner.lock().unwrap();
        let (data, pos) = &mut *g;
        let n = num_bytes.max(0) as usize;
        if data.len() < *pos + n {
            data.resize(*pos + n, 0);
        }
        std::ptr::copy_nonoverlapping(buffer as *const u8, data.as_mut_ptr().add(*pos), n);
        *pos += n;
        if !num_written.is_null() {
            *num_written = n as int32;
        }
        kResultOk
    }
    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let mut g = self.inner.lock().unwrap();
        let (data, cur) = &mut *g;
        let base = match mode {
            m if m == IBStream_::IStreamSeekMode_::kIBSeekSet as int32 => 0i64,
            m if m == IBStream_::IStreamSeekMode_::kIBSeekCur as int32 => *cur as i64,
            m if m == IBStream_::IStreamSeekMode_::kIBSeekEnd as int32 => data.len() as i64,
            _ => return kInvalidArgument,
        };
        let new = (base + pos).max(0) as usize;
        *cur = new;
        if !result.is_null() {
            *result = new as int64;
        }
        kResultOk
    }
    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if !pos.is_null() {
            *pos = self.inner.lock().unwrap().1 as int64;
        }
        kResultOk
    }
}

struct EventList {
    events: Mutex<Vec<Event>>,
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.events.lock().unwrap().len() as int32
    }
    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        match self.events.lock().unwrap().get(index as usize) {
            Some(ev) => {
                *e = *ev;
                kResultOk
            }
            None => kInvalidArgument,
        }
    }
    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        self.events.lock().unwrap().push(*e);
        kResultOk
    }
}

struct ParamQueue {
    data: Mutex<(ParamID, Vec<(int32, ParamValue)>)>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.data.lock().unwrap().0
    }
    unsafe fn getPointCount(&self) -> int32 {
        self.data.lock().unwrap().1.len() as int32
    }
    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        match self.data.lock().unwrap().1.get(index as usize) {
            Some(&(o, v)) => {
                *offset = o;
                *value = v;
                kResultOk
            }
            None => kInvalidArgument,
        }
    }
    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let mut g = self.data.lock().unwrap();
        g.1.push((offset, value));
        if !index.is_null() {
            *index = g.1.len() as int32 - 1;
        }
        kResultOk
    }
}

const MAX_PARAM_QUEUES: usize = 128;

struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    used: Mutex<usize>,
}

impl ParamChanges {
    fn new() -> Self {
        let queues = (0..MAX_PARAM_QUEUES).map(|_| ComWrapper::new(ParamQueue { data: Mutex::new((0, Vec::with_capacity(8))) })).collect();
        Self { queues, used: Mutex::new(0) }
    }

    /// Loads (id, value) pairs; one point at offset 0 per parameter.
    fn fill(&self, changes: &[(u32, f64)]) {
        let mut used = 0;
        for &(id, value) in changes {
            let existing = self.queues[..used].iter().find(|q| q.data.lock().unwrap().0 == id);
            let q = match existing {
                Some(q) => q,
                None if used < self.queues.len() => {
                    let q = &self.queues[used];
                    used += 1;
                    let mut d = q.data.lock().unwrap();
                    d.0 = id;
                    d.1.clear();
                    drop(d);
                    q
                }
                None => break,
            };
            let mut d = q.data.lock().unwrap();
            d.1.clear();
            d.1.push((0, value));
        }
        *self.used.lock().unwrap() = used;
    }
}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        *self.used.lock().unwrap() as int32
    }
    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if (index as usize) < *self.used.lock().unwrap() {
            self.queues[index as usize].as_com_ref::<IParamValueQueue>().unwrap().as_ptr()
        } else {
            std::ptr::null_mut()
        }
    }
    unsafe fn addParameterData(&self, _id: *const ParamID, _index: *mut int32) -> *mut IParamValueQueue {
        std::ptr::null_mut()
    }
}

/// Receives parameter edits made in the plugin's editor.
struct ComponentHandler {
    pending: Arc<Mutex<Vec<(u32, f64)>>>,
    /// Last parameter edited in the editor: (sequence << 32) | parameter id.
    touched: Arc<AtomicU64>,
}

/// Orders edits across all plugins, to find the most recently tweaked parameter.
static TOUCH_SEQ: AtomicU32 = AtomicU32::new(0);

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        let seq = TOUCH_SEQ.fetch_add(1, Ordering::Relaxed) as u64 + 1;
        self.touched.store(seq << 32 | id as u64, Ordering::Relaxed);
        if let Ok(mut p) = self.pending.lock() {
            if let Some(e) = p.iter_mut().find(|e| e.0 == id) {
                e.1 = value;
            } else {
                p.push((id, value));
            }
        }
        kResultOk
    }
    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        kResultOk
    }
}

// ---------------------------------------------------------------------------
// Plugin instance
// ---------------------------------------------------------------------------

const MAX_BLOCK: usize = 2048;

struct Bus {
    channels: usize,
    bufs: Vec<Vec<f32>>,
    ptrs: Vec<*mut f32>,
}

impl Bus {
    fn new(channels: usize) -> Self {
        let mut bufs: Vec<Vec<f32>> = (0..channels).map(|_| vec![0.0; MAX_BLOCK]).collect();
        let ptrs = bufs.iter_mut().map(|b| b.as_mut_ptr()).collect();
        Self { channels, bufs, ptrs }
    }
}

struct Processor {
    processor: ComPtr<IAudioProcessor>,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    in_abb: Vec<AudioBusBuffers>,
    out_abb: Vec<AudioBusBuffers>,
    events: ComWrapper<EventList>,
    params: ComWrapper<ParamChanges>,
    param_scratch: Vec<(u32, f64)>,
    ctx: ProcessContext,
    has_event_input: bool,
}

pub struct PluginInstance {
    pub info: PluginInfo,
    proc: Mutex<Processor>,
    component: ComPtr<IComponent>,
    controller: Option<ComPtr<IEditController>>,
    connections: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    pending: Arc<Mutex<Vec<(u32, f64)>>>,
    _handler: ComWrapper<ComponentHandler>,
    _host: ComWrapper<HostApp>,
    pub latency: u32,
    /// VST3 parameter ids by index (FL addresses plugin parameters by index).
    param_ids: Vec<u32>,
    touched: Arc<AtomicU64>,
    /// Parameter id for each MIDI controller (0..=127 CCs, 128 aftertouch,
    /// 129 pitch bend), from the plugin's IMidiMapping.
    midi_map: Vec<Option<u32>>,
}

// VST3 objects are used following the spec's threading rules: processing on the
// audio thread behind `proc`, the controller/editor on the UI thread.
unsafe impl Send for PluginInstance {}
unsafe impl Sync for PluginInstance {}

impl std::fmt::Debug for PluginInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PluginInstance({})", self.info.name)
    }
}

impl PluginInstance {
    pub fn new(info: &PluginInfo, sample_rate: f64) -> Result<Arc<Self>> {
        let module = load_module(&info.path)?;
        let cid = hex_tuid(&info.class_id)?;
        unsafe {
            let host = ComWrapper::new(HostApp);
            let host_unknown = host.as_com_ref::<IHostApplication>().unwrap().as_ptr() as *mut FUnknown;

            let mut obj: *mut c_void = std::ptr::null_mut();
            let r = module.factory.createInstance(cid.as_ptr(), IComponent::IID.as_ptr() as FIDString, &mut obj);
            let component = (r == kResultOk).then(|| ComPtr::from_raw(obj as *mut IComponent)).flatten().ok_or_else(|| anyhow!("could not create {}", info.name))?;
            if component.initialize(host_unknown) != kResultOk {
                bail!("{} failed to initialize", info.name);
            }
            let processor = component.cast::<IAudioProcessor>().ok_or_else(|| anyhow!("{} has no audio processor", info.name))?;

            // Controller: either the component itself or a separate class.
            let mut separate = false;
            let controller = match component.cast::<IEditController>() {
                Some(c) => Some(c),
                None => {
                    let mut ccid: TUID = [0; 16];
                    if component.getControllerClassId(&mut ccid) == kResultOk {
                        let mut obj: *mut c_void = std::ptr::null_mut();
                        let r = module.factory.createInstance(ccid.as_ptr(), IEditController::IID.as_ptr() as FIDString, &mut obj);
                        let c = (r == kResultOk).then(|| ComPtr::from_raw(obj as *mut IEditController)).flatten();
                        if let Some(c) = &c {
                            c.initialize(host_unknown);
                            separate = true;
                        }
                        c
                    } else {
                        None
                    }
                }
            };

            let connections = match (&controller, separate) {
                (Some(c), true) => match (component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>()) {
                    (Some(a), Some(b)) => {
                        a.connect(b.as_ptr());
                        b.connect(a.as_ptr());
                        Some((a, b))
                    }
                    _ => None,
                },
                _ => None,
            };

            let pending = Arc::new(Mutex::new(Vec::new()));
            let touched = Arc::new(AtomicU64::new(0));
            let handler = ComWrapper::new(ComponentHandler { pending: pending.clone(), touched: touched.clone() });
            if let Some(c) = &controller {
                c.setComponentHandler(handler.as_com_ref::<IComponentHandler>().unwrap().as_ptr());
                // Sync the controller with the component's initial state.
                let s = MemStream::new(Vec::new());
                let sp = s.as_com_ref::<IBStream>().unwrap().as_ptr();
                if component.getState(sp) == kResultOk {
                    s.seek(0, IBStream_::IStreamSeekMode_::kIBSeekSet as int32, std::ptr::null_mut());
                    c.setComponentState(sp);
                }
            }

            // Buses: stereo main in/out, other audio buses inactive.
            let audio = MediaTypes_::kAudio as MediaType;
            let (input_dir, output_dir) = (BusDirections_::kInput as BusDirection, BusDirections_::kOutput as BusDirection);
            let n_in = component.getBusCount(audio, input_dir).max(0) as usize;
            let n_out = component.getBusCount(audio, output_dir).max(0) as usize;
            let mut in_arr: Vec<SpeakerArrangement> = (0..n_in)
                .map(|i| {
                    let mut a: SpeakerArrangement = 0;
                    processor.getBusArrangement(input_dir, i as int32, &mut a);
                    if i == 0 { SpeakerArr::kStereo } else { a }
                })
                .collect();
            let mut out_arr: Vec<SpeakerArrangement> = (0..n_out)
                .map(|i| {
                    let mut a: SpeakerArrangement = 0;
                    processor.getBusArrangement(output_dir, i as int32, &mut a);
                    if i == 0 { SpeakerArr::kStereo } else { a }
                })
                .collect();
            processor.setBusArrangements(in_arr.as_mut_ptr(), n_in as int32, out_arr.as_mut_ptr(), n_out as int32);
            let channels = |dir: BusDirection, i: usize| -> usize {
                let mut a: SpeakerArrangement = 0;
                processor.getBusArrangement(dir, i as int32, &mut a);
                (a.count_ones() as usize).max(1)
            };
            let inputs: Vec<Bus> = (0..n_in).map(|i| Bus::new(channels(input_dir, i))).collect();
            let outputs: Vec<Bus> = (0..n_out).map(|i| Bus::new(channels(output_dir, i))).collect();
            for i in 0..n_in {
                component.activateBus(audio, input_dir, i as int32, (i == 0) as TBool);
            }
            for i in 0..n_out {
                component.activateBus(audio, output_dir, i as int32, (i == 0) as TBool);
            }
            let event = MediaTypes_::kEvent as MediaType;
            let has_event_input = component.getBusCount(event, input_dir) > 0;
            if has_event_input {
                component.activateBus(event, input_dir, 0, 1);
            }

            let mut setup = ProcessSetup {
                processMode: ProcessModes_::kRealtime as int32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
                maxSamplesPerBlock: MAX_BLOCK as int32,
                sampleRate: sample_rate,
            };
            processor.setupProcessing(&mut setup);
            component.setActive(1);
            processor.setProcessing(1);
            let latency = processor.getLatencySamples();
            let midi_map: Vec<Option<u32>> = match controller.as_ref().and_then(|c| c.cast::<IMidiMapping>()) {
                Some(m) => (0..130)
                    .map(|cc| {
                        let mut id: ParamID = 0;
                        (m.getMidiControllerAssignment(0, 0, cc as CtrlNumber, &mut id) == kResultOk).then_some(id)
                    })
                    .collect(),
                None => vec![None; 130],
            };
            let param_ids: Vec<u32> = match &controller {
                Some(c) => (0..c.getParameterCount())
                    .filter_map(|i| {
                        let mut info: ParameterInfo = std::mem::zeroed();
                        (c.getParameterInfo(i, &mut info) == kResultOk).then_some(info.id)
                    })
                    .collect(),
                None => Vec::new(),
            };

            let mk_abb = |buses: &mut Vec<Bus>| -> Vec<AudioBusBuffers> {
                buses
                    .iter_mut()
                    .map(|b| AudioBusBuffers {
                        numChannels: b.channels as int32,
                        silenceFlags: 0,
                        __field0: AudioBusBuffers__type0 { channelBuffers32: b.ptrs.as_mut_ptr() },
                    })
                    .collect()
            };
            let mut inputs = inputs;
            let mut outputs = outputs;
            let in_abb = mk_abb(&mut inputs);
            let out_abb = mk_abb(&mut outputs);
            let mut ctx: ProcessContext = std::mem::zeroed();
            ctx.sampleRate = sample_rate;

            let proc = Processor {
                processor,
                inputs,
                outputs,
                in_abb,
                out_abb,
                events: ComWrapper::new(EventList { events: Mutex::new(Vec::with_capacity(512)) }),
                params: ComWrapper::new(ParamChanges::new()),
                param_scratch: Vec::with_capacity(MAX_PARAM_QUEUES),
                ctx,
                has_event_input,
            };
            Ok(Arc::new(Self {
                info: info.clone(),
                proc: Mutex::new(proc),
                component,
                controller,
                connections,
                pending,
                _handler: handler,
                _host: host,
                latency,
                param_ids,
                touched,
                midi_map,
            }))
        }
    }


    /// Queues a parameter change by index (normalized 0..=1). Real-time safe:
    /// skipped if the queue is busy.
    pub fn set_param_index(&self, index: u32, value: f64) {
        let Some(&id) = self.param_ids.get(index as usize) else { return };
        self.queue_param(id, value);
    }

    /// Sends a MIDI controller (CC number, or 129 = pitch bend) as the
    /// parameter the plugin maps it to. Returns false if it maps none.
    /// Real-time safe.
    pub fn midi_controller(&self, controller: u8, value: f64) -> bool {
        match self.midi_map.get(controller as usize).copied().flatten() {
            Some(id) => {
                self.queue_param(id, value.clamp(0.0, 1.0));
                true
            }
            None => false,
        }
    }

    fn queue_param(&self, id: u32, value: f64) {
        if let Ok(mut p) = self.pending.try_lock() {
            if let Some(e) = p.iter_mut().find(|e| e.0 == id) {
                e.1 = value;
            } else if p.len() < 256 {
                p.push((id, value));
            }
        }
    }

    /// Sets a parameter from the UI thread (e.g. a MIDI controller): the
    /// processor gets the change and the editor shows it.
    pub fn set_param_from_ui(&self, index: u32, value: f64) {
        self.set_param_index(index, value);
        if let (Some(c), Some(&id)) = (&self.controller, self.param_ids.get(index as usize)) {
            unsafe {
                c.setParamNormalized(id, value);
            }
        }
    }

    /// The parameter most recently edited in this plugin's editor, as
    /// (global edit sequence, parameter index). Higher sequence = more recent.
    pub fn last_touched(&self) -> Option<(u32, u32)> {
        let v = self.touched.load(Ordering::Relaxed);
        let (seq, id) = ((v >> 32) as u32, v as u32);
        let index = self.param_ids.iter().position(|&p| p == id)?;
        (seq > 0).then_some((seq, index as u32))
    }

    /// Display name of a parameter.
    pub fn param_name(&self, index: u32) -> Option<String> {
        let c = self.controller.as_ref()?;
        unsafe {
            let mut info: ParameterInfo = std::mem::zeroed();
            if c.getParameterInfo(index as int32, &mut info) != kResultOk {
                return None;
            }
            let units: Vec<u16> = info.title.iter().take_while(|&&ch| ch != 0).map(|&ch| ch as u16).collect();
            Some(String::from_utf16_lossy(&units))
        }
    }

    pub fn has_editor(&self) -> bool {
        self.controller.is_some()
    }

    /// Processes `n` frames. `io_l`/`io_r` hold the input (for effects) and
    /// receive the output. Returns false if the plugin was busy (output untouched).
    pub fn process(&self, n: usize, events: &[NoteEvent], io_l: &mut [f32], io_r: &mut [f32], time: &TimeInfo) -> bool {
        let Ok(mut p) = self.proc.try_lock() else { return false };
        let p = &mut *p;
        let n = n.min(MAX_BLOCK).min(io_l.len()).min(io_r.len());
        unsafe {
            // Audio input on the main bus, silence elsewhere.
            for (bi, bus) in p.inputs.iter_mut().enumerate() {
                for (ci, buf) in bus.bufs.iter_mut().enumerate() {
                    let src: Option<&[f32]> = match (bi, ci) {
                        (0, 0) => Some(&io_l[..n]),
                        (0, 1) => Some(&io_r[..n]),
                        _ => None,
                    };
                    match src {
                        Some(s) => buf[..n].copy_from_slice(s),
                        None => buf[..n].fill(0.0),
                    }
                }
            }

            // Events.
            {
                let mut ev = p.events.events.lock().unwrap();
                ev.clear();
                if p.has_event_input {
                    for e in events {
                        let mut out: Event = std::mem::zeroed();
                        out.busIndex = 0;
                        out.sampleOffset = (e.offset() as usize).min(n.saturating_sub(1)) as int32;
                        match *e {
                            NoteEvent::On { key, velocity, .. } => {
                                out.r#type = Event_::EventTypes_::kNoteOnEvent as uint16;
                                out.__field0.noteOn = NoteOnEvent { channel: 0, pitch: key as int16, tuning: 0.0, velocity, length: 0, noteId: -1 };
                            }
                            NoteEvent::Off { key, .. } => {
                                out.r#type = Event_::EventTypes_::kNoteOffEvent as uint16;
                                out.__field0.noteOff = NoteOffEvent { channel: 0, pitch: key as int16, velocity: 0.0, noteId: -1, tuning: 0.0 };
                            }
                        }
                        ev.push(out);
                    }
                }
            }

            // Parameter edits from the editor.
            p.param_scratch.clear();
            if let Ok(mut pending) = self.pending.try_lock() {
                p.param_scratch.extend(pending.drain(..));
            }
            p.params.fill(&p.param_scratch);

            // Transport.
            use ProcessContext_::StatesAndFlags_::*;
            p.ctx.state = (kTempoValid | kTimeSigValid | kProjectTimeMusicValid | if time.playing { kPlaying } else { 0 }) as uint32;
            p.ctx.sampleRate = time.sample_rate;
            p.ctx.tempo = time.tempo;
            p.ctx.projectTimeMusic = time.ppq;
            p.ctx.projectTimeSamples = time.project_samples;
            p.ctx.timeSigNumerator = time.time_sig.0 as int32;
            p.ctx.timeSigDenominator = time.time_sig.1 as int32;

            let mut data = ProcessData {
                processMode: ProcessModes_::kRealtime as int32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
                numSamples: n as int32,
                numInputs: p.in_abb.len() as int32,
                numOutputs: p.out_abb.len() as int32,
                inputs: if p.in_abb.is_empty() { std::ptr::null_mut() } else { p.in_abb.as_mut_ptr() },
                outputs: if p.out_abb.is_empty() { std::ptr::null_mut() } else { p.out_abb.as_mut_ptr() },
                inputParameterChanges: p.params.as_com_ref::<IParameterChanges>().unwrap().as_ptr(),
                outputParameterChanges: std::ptr::null_mut(),
                inputEvents: p.events.as_com_ref::<IEventList>().unwrap().as_ptr(),
                outputEvents: std::ptr::null_mut(),
                processContext: &mut p.ctx,
            };
            p.processor.process(&mut data);

            match p.outputs.first() {
                Some(out) if out.channels >= 2 => {
                    io_l[..n].copy_from_slice(&out.bufs[0][..n]);
                    io_r[..n].copy_from_slice(&out.bufs[1][..n]);
                }
                Some(out) if out.channels == 1 => {
                    io_l[..n].copy_from_slice(&out.bufs[0][..n]);
                    io_r[..n].copy_from_slice(&out.bufs[0][..n]);
                }
                _ => {
                    io_l[..n].fill(0.0);
                    io_r[..n].fill(0.0);
                }
            }
        }
        true
    }

    /// Component + controller state in Goaty Loops's container format.
    pub fn get_state(&self) -> Vec<u8> {
        let _guard = self.proc.lock().unwrap();
        unsafe {
            let comp = MemStream::new(Vec::new());
            self.component.getState(comp.as_com_ref::<IBStream>().unwrap().as_ptr());
            let ctrl = MemStream::new(Vec::new());
            if let Some(c) = &self.controller {
                c.getState(ctrl.as_com_ref::<IBStream>().unwrap().as_ptr());
            }
            let (comp, ctrl) = (comp.data(), ctrl.data());
            let mut out = Vec::with_capacity(12 + comp.len() + ctrl.len());
            out.extend_from_slice(b"BCV3");
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&comp);
            out.extend_from_slice(&(ctrl.len() as u32).to_le_bytes());
            out.extend_from_slice(&ctrl);
            out
        }
    }

    /// Restores state saved by [`get_state`]; anything else is treated as a raw
    /// component state. Returns whether the plugin accepted it.
    pub fn set_state(&self, data: &[u8]) -> bool {
        let (comp, ctrl) = parse_state(data);
        if comp.is_empty() {
            return false;
        }
        let _guard = self.proc.lock().unwrap();
        unsafe {
            let s = MemStream::new(comp.to_vec());
            let sp = s.as_com_ref::<IBStream>().unwrap().as_ptr();
            let ok = self.component.setState(sp) == kResultOk;
            if let Some(c) = &self.controller {
                s.seek(0, IBStream_::IStreamSeekMode_::kIBSeekSet as int32, std::ptr::null_mut());
                c.setComponentState(sp);
                if !ctrl.is_empty() {
                    let s2 = MemStream::new(ctrl.to_vec());
                    c.setState(s2.as_com_ref::<IBStream>().unwrap().as_ptr());
                }
            }
            ok
        }
    }

    /// Creates the plugin's editor view (UI thread only).
    pub fn create_view(&self) -> Option<ComPtr<IPlugView>> {
        let c = self.controller.as_ref()?;
        unsafe {
            let v = c.createView(ViewType::kEditor);
            ComPtr::from_raw(v)
        }
    }
}

fn parse_state(data: &[u8]) -> (&[u8], &[u8]) {
    if data.len() >= 8 && &data[..4] == b"BCV3" {
        let cl = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        if 8 + cl + 4 <= data.len() {
            let comp = &data[8..8 + cl];
            let tl = u32::from_le_bytes(data[8 + cl..12 + cl].try_into().unwrap()) as usize;
            let ctrl = data.get(12 + cl..12 + cl + tl).unwrap_or(&[]);
            return (comp, ctrl);
        }
    }
    (data, &[])
}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        unsafe {
            if let Ok(p) = self.proc.lock() {
                p.processor.setProcessing(0);
            }
            self.component.setActive(0);
            if let Some((a, b)) = &self.connections {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = &self.controller {
                c.setComponentHandler(std::ptr::null_mut());
                if self.connections.is_some() {
                    c.terminate();
                }
            }
            self.component.terminate();
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin database (scan results cache)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginDb {
    pub plugins: Vec<PluginInfo>,
    /// Bundles that failed to scan, with the error.
    pub failed: Vec<(PathBuf, String)>,
    /// Modification time (seconds) per scanned bundle.
    pub scanned: HashMap<PathBuf, u64>,
}

fn mtime(p: &Path) -> u64 {
    let bin = module_binary(p);
    std::fs::metadata(&bin)
        .or_else(|_| std::fs::metadata(p))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl PluginDb {
    pub fn path() -> PathBuf {
        crate::paths::app_data_dir().join("plugins.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn instruments(&self) -> impl Iterator<Item = &PluginInfo> {
        self.plugins.iter().filter(|p| p.instrument)
    }

    pub fn effects(&self) -> impl Iterator<Item = &PluginInfo> {
        self.plugins.iter().filter(|p| !p.instrument)
    }

    /// Finds an installed plugin by bundle path, file name or plugin name.
    pub fn find(&self, path: Option<&str>, name: Option<&str>) -> Option<&PluginInfo> {
        let file = |p: &str| p.rsplit(['\\', '/']).next().map(|s| s.to_ascii_lowercase());
        if let Some(path) = path {
            let want = file(path);
            if let Some(p) = self.plugins.iter().find(|p| p.path.to_string_lossy().eq_ignore_ascii_case(path)) {
                return Some(p);
            }
            if let Some(p) = self.plugins.iter().find(|p| p.path.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()) == want) {
                return Some(p);
            }
        }
        let name = name?.to_ascii_lowercase();
        self.plugins.iter().find(|p| p.name.to_ascii_lowercase() == name)
    }

    /// Scans new or changed bundles. `scan_one` performs the actual scan
    /// (typically in a child process). Returns the number of bundles scanned.
    pub fn refresh(&mut self, dirs: &[PathBuf], mut scan_one: impl FnMut(&Path) -> Result<Vec<PluginInfo>>) -> usize {
        let bundles = find_bundles(dirs);
        self.plugins.retain(|p| bundles.contains(&p.path));
        self.failed.retain(|(p, _)| bundles.contains(p));
        self.scanned.retain(|p, _| bundles.contains(p));
        let mut n = 0;
        for b in bundles {
            let t = mtime(&b);
            if self.scanned.get(&b) == Some(&t) {
                continue;
            }
            n += 1;
            self.plugins.retain(|p| p.path != b);
            self.failed.retain(|(p, _)| p != &b);
            match scan_one(&b) {
                Ok(list) => self.plugins.extend(list),
                Err(e) => self.failed.push((b.clone(), e.to_string())),
            }
            self.scanned.insert(b, t);
        }
        self.plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        n
    }
}

/// Serde helper storing plugin state as base64 text.
pub mod state_serde {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(s).map_err(serde::de::Error::custom)
    }
}
