use memmap2::MmapMut;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const SHM_MAGIC: u64 = 0x4255_434B_594F_534B;
const SHM_VERSION: u32 = 2; // bumped: layout changed (AtomicU64 slot.seq, no ring lock)

const ENTRY_FREE: u8 = 0;
const ENTRY_INIT: u8 = 1;
const ENTRY_READY: u8 = 2;

pub const DEFAULT_RINGBUFFER_PATH_ENV: &str = "BUCKYOS_KEVENT_RINGBUFFER_PATH";
const DEFAULT_RINGBUFFER_PATH: &str = "/tmp/buckyos_kevent_ringbuffer_v2.shm";

const MAX_RINGS: usize = 16;
const RING_CAPACITY: usize = 512; // must be power of 2
const SLOT_DATA_SIZE: usize = 2048;
const DIRTY_WORDS: usize = (MAX_RINGS + 63) / 64;

// ---------------------------------------------------------------------------
// Shared memory layout  (all repr(C) + align(64) for cache-line isolation)
// ---------------------------------------------------------------------------

#[repr(C, align(64))]
struct SharedHeader {
    magic: u64,
    version: u32,
    max_rings: u32,
    ring_capacity: u32,
    slot_data_size: u32,
    init_lock: AtomicU8,
    _pad0: [u8; 7],
    epoch: AtomicU64,
    notify_seq: AtomicU64,
    dirty_mask: [AtomicU64; DIRTY_WORDS],
}

#[repr(C, align(64))]
struct RingDirectoryEntry {
    state: AtomicU8,
    _pad0: [u8; 3],
    owner_pid: AtomicU32,
    generation: AtomicU64,
    last_heartbeat_ms: AtomicU64,
    ring_offset: AtomicU32,
    ring_bytes: AtomicU32,
}

/// A single slot inside a PublishRing.
///
/// `seq` is an AtomicU64 and serves as the commit / validity marker.
/// The producer writes payload+len first, then does a **release store**
/// on `seq`.  Consumers do an **acquire load** of `seq`, copy the payload,
/// then re-check `seq` (seqlock double-check) to detect torn reads caused
/// by the producer wrapping around and overwriting this slot mid-read.
#[repr(C, align(64))]
struct RingSlot {
    seq: AtomicU64,
    len: AtomicU32,
    _pad0: u32,
    payload: [u8; SLOT_DATA_SIZE],
}

/// Per-producer ring.  There is exactly **one writer** (the owning process)
/// so `head_seq` advances without any CAS.  Consumers only read.
///
/// No lock is needed:
/// - Writer path: memcpy payload → store_release(slot.seq) → store_release(head_seq)
/// - Reader path: load_acquire(head_seq) → load_acquire(slot.seq) → memcpy → re-load(slot.seq)
#[repr(C, align(64))]
struct PublishRing {
    head_seq: AtomicU64,
    _pad0: [u8; 56], // fill rest of cache line
    slots: [RingSlot; RING_CAPACITY],
}

#[repr(C)]
struct SharedRegion {
    header: SharedHeader,
    directory: [RingDirectoryEntry; MAX_RINGS],
    rings: [PublishRing; MAX_RINGS],
}

// ---------------------------------------------------------------------------
// Per-consumer cursor (lives in process-private memory)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct RingCursor {
    generation: u64,
    read_seq: u64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Handle to the shared-memory event ring buffer.
///
/// Internally splits publish state and consume state so that the publish
/// fast-path never blocks on drain (and vice-versa).
pub struct SharedKEventRingBuffer {
    /// Publish-side state: only needs the mmap pointer and our ring id.
    /// Protected by its own mutex so publish never contends with drain.
    publish: Mutex<PublishInner>,

    /// Consume-side state: cursors into other processes' rings.
    consume: Mutex<ConsumeInner>,
}

struct PublishInner {
    mmap: MmapMut,
    my_ring_id: usize,
}

struct ConsumeInner {
    /// We re-use the *same* mmap (via raw pointer) – the MmapMut is owned
    /// by PublishInner.  This is safe because we never unmap while either
    /// mutex is alive, and SharedRegion fields are either atomic or only
    /// written by a single producer.
    region_ptr: *mut SharedRegion,
    my_ring_id: usize,
    cursors: HashMap<usize, RingCursor>,
}

// SAFETY: the mmap region is process-wide; sharing the pointer across
// threads within the same process is fine (all accesses go through atomics
// or the seqlock double-check protocol).
unsafe impl Send for ConsumeInner {}

impl SharedKEventRingBuffer {
    pub fn open() -> Result<Self, String> {
        let path = ringbuffer_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("create ringbuffer dir {} failed: {}", parent.display(), e)
            })?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|e| format!("open ringbuffer file {} failed: {}", path.display(), e))?;

        let region_len = size_of::<SharedRegion>() as u64;
        file.set_len(region_len)
            .map_err(|e| format!("resize ringbuffer file failed: {}", e))?;

        let mmap = unsafe {
            MmapMut::map_mut(&file)
                .map_err(|e| format!("mmap ringbuffer file failed: {}", e))?
        };

        let region_ptr = mmap.as_ptr() as *mut SharedRegion;
        let region = unsafe { &mut *region_ptr };
        initialize_region_if_needed(region);

        let pid = std::process::id();
        let my_ring_id = allocate_ring(region, pid)
            .ok_or_else(|| format!("no free ring entry in {}", path.display()))?;

        Ok(Self {
            publish: Mutex::new(PublishInner { mmap, my_ring_id }),
            consume: Mutex::new(ConsumeInner {
                region_ptr,
                my_ring_id,
                cursors: HashMap::new(),
            }),
        })
    }

    /// Current epoch value (daemon increments on restart; SDK can poll this).
    pub fn epoch(&self) -> u64 {
        let inner = self.publish.lock().unwrap();
        let region = unsafe { &*(inner.mmap.as_ptr() as *const SharedRegion) };
        region.header.epoch.load(Ordering::Acquire)
    }

    /// Increment epoch (called by Node Daemon on startup).
    pub fn bump_epoch(&self) {
        let inner = self.publish.lock().unwrap();
        let region = unsafe { &*(inner.mmap.as_ptr() as *const SharedRegion) };
        region.header.epoch.fetch_add(1, Ordering::AcqRel);
    }

    // -----------------------------------------------------------------------
    // Publish path  (lock-free on the shared-memory side)
    // -----------------------------------------------------------------------

    pub fn publish_event<T: Serialize>(&self, event: &T) -> Result<(), String> {
        let bytes = serde_json::to_vec(event)
            .map_err(|e| format!("encode event failed: {}", e))?;
        self.publish_payload(&bytes)
    }

    fn publish_payload(&self, payload: &[u8]) -> Result<(), String> {
        if payload.is_empty() {
            return Ok(());
        }
        if payload.len() > SLOT_DATA_SIZE {
            return Err(format!(
                "payload too large: {} > {}",
                payload.len(),
                SLOT_DATA_SIZE
            ));
        }

        let inner = self.publish.lock()
            .map_err(|_| "publish lock poisoned".to_string())?;
        let region = unsafe { &*(inner.mmap.as_ptr() as *const SharedRegion) };
        let ring = &region.rings[inner.my_ring_id];

        // --- single-producer write: no CAS needed ---
        let prev_seq = ring.head_seq.load(Ordering::Relaxed);
        let seq = prev_seq.wrapping_add(1);
        let idx = (seq as usize) & (RING_CAPACITY - 1);
        let slot = &ring.slots[idx];

        // 1) Invalidate the slot so concurrent readers see a mismatch
        slot.seq.store(0, Ordering::Relaxed);

        // 2) Write payload + len  (ordinary writes, no atomics needed for
        //    these because the seq release-store below is the publish barrier)
        //    SAFETY: we are the sole writer of this ring; the slot memory is
        //    only read by consumers *after* they observe `slot.seq == target`
        //    via an acquire load, and they re-check afterwards.
        unsafe {
            let slot_ptr = slot as *const RingSlot as *mut RingSlot;
            let dst = (*slot_ptr).payload.as_mut_ptr();
            std::ptr::copy_nonoverlapping(payload.as_ptr(), dst, payload.len());
            (*slot_ptr)._pad0 = 0;
        }
        slot.len.store(payload.len() as u32, Ordering::Relaxed);

        // 3) Commit point: release-store seq so consumers see payload+len
        slot.seq.store(seq, Ordering::Release);

        // 4) Advance head_seq (must be AFTER slot.seq per design doc §3.2)
        ring.head_seq.store(seq, Ordering::Release);

        // 5) Update heartbeat
        let entry = &region.directory[inner.my_ring_id];
        entry.last_heartbeat_ms.store(now_millis(), Ordering::Relaxed);

        // 6) Set dirty bit + bump notify_seq for futex wake
        let word_idx = inner.my_ring_id / 64;
        let bit = 1u64 << (inner.my_ring_id % 64);
        let old = region.header.dirty_mask[word_idx].fetch_or(bit, Ordering::AcqRel);
        // Always bump notify_seq so waiters see a change; only the 0→1
        // transition would need a futex_wake in a full implementation.
        region.header.notify_seq.fetch_add(1, Ordering::AcqRel);
        let _ = old; // futex_wake would go here in production

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Consume path  (lock-free reads with seqlock double-check)
    // -----------------------------------------------------------------------

    pub fn drain_events<T: DeserializeOwned>(&self, max_events: usize) -> Vec<T> {
        let payloads = self.drain_payloads(max_events);
        payloads
            .into_iter()
            .filter_map(|p| serde_json::from_slice::<T>(&p).ok())
            .collect()
    }

    fn drain_payloads(&self, max_events: usize) -> Vec<Vec<u8>> {
        if max_events == 0 {
            return Vec::new();
        }

        let mut inner = match self.consume.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };

        let region = unsafe { &*inner.region_ptr };
        let my_ring_id = inner.my_ring_id;
        let mut out = Vec::with_capacity(max_events.min(64));

        // --- use dirty_bitmap to only scan rings that have new data ---
        let mut scan_mask = [0u64; DIRTY_WORDS];
        for w in 0..DIRTY_WORDS {
            // exchange → 0: we take ownership of the dirty bits
            scan_mask[w] = region.header.dirty_mask[w].swap(0, Ordering::AcqRel);
        }

        for ring_id in 0..MAX_RINGS {
            if out.len() >= max_events {
                break;
            }
            if ring_id == my_ring_id {
                continue;
            }

            // Skip rings not marked dirty
            let word_idx = ring_id / 64;
            let bit = 1u64 << (ring_id % 64);
            if scan_mask[word_idx] & bit == 0 {
                continue;
            }

            let entry = &region.directory[ring_id];
            if entry.state.load(Ordering::Acquire) != ENTRY_READY {
                continue;
            }

            let generation = entry.generation.load(Ordering::Acquire);
            let ring = &region.rings[ring_id];
            let cursor = inner.cursors.entry(ring_id).or_insert_with(|| RingCursor {
                generation,
                read_seq: ring.head_seq.load(Ordering::Acquire),
            });

            // Generation changed → ring was recycled; reset cursor
            if cursor.generation != generation {
                cursor.generation = generation;
                cursor.read_seq = ring.head_seq.load(Ordering::Acquire);
                continue;
            }

            while out.len() < max_events {
                match consume_one_payload(ring, cursor) {
                    Some(payload) => out.push(payload),
                    None => break,
                }
            }
        }

        // Update our own heartbeat on the consume path too, so a process
        // that only subscribes (never publishes) doesn't get reaped.
        let my_entry = &region.directory[my_ring_id];
        my_entry.last_heartbeat_ms.store(now_millis(), Ordering::Relaxed);

        out
    }
}

// ---------------------------------------------------------------------------
// Lock-free consume with seqlock double-check  (design doc §5.3)
// ---------------------------------------------------------------------------

fn consume_one_payload(ring: &PublishRing, cursor: &mut RingCursor) -> Option<Vec<u8>> {
    let head = ring.head_seq.load(Ordering::Acquire);
    if cursor.read_seq == head {
        return None;
    }

    // Consumer fell behind: skip to the oldest still-available slot
    if head.wrapping_sub(cursor.read_seq) > RING_CAPACITY as u64 {
        cursor.read_seq = head.wrapping_sub(RING_CAPACITY as u64);
    }

    let target = cursor.read_seq.wrapping_add(1);
    let idx = (target as usize) & (RING_CAPACITY - 1);
    let slot = &ring.slots[idx];

    // --- first check: is the slot committed with the sequence we expect? ---
    let s1 = slot.seq.load(Ordering::Acquire);
    if s1 != target {
        if s1 > target {
            // Slot was overwritten with a newer event; jump forward
            cursor.read_seq = s1.wrapping_sub(1);
        }
        // else: slot not yet committed (tiny window); try again later
        return None;
    }

    // --- copy payload under the protection of the seqlock ---
    let len = slot.len.load(Ordering::Relaxed) as usize;
    if len == 0 || len > SLOT_DATA_SIZE {
        cursor.read_seq = target;
        return None;
    }
    let payload = slot.payload[..len].to_vec();

    // --- second check: was the slot overwritten while we were copying? ---
    let s2 = slot.seq.load(Ordering::Acquire);
    if s2 != target {
        // Torn read – discard this payload; cursor stays so we retry
        // from the new position next time.
        if s2 > target {
            cursor.read_seq = s2.wrapping_sub(1);
        }
        return None;
    }

    cursor.read_seq = target;
    Some(payload)
}

// ---------------------------------------------------------------------------
// Shared-memory initialization
// ---------------------------------------------------------------------------

/// Racy but acceptable for best-effort EventBus: in the worst case two
/// processes reset the region concurrently and a few events are lost.
fn initialize_region_if_needed(region: &mut SharedRegion) {
    if header_matches(region) {
        return;
    }

    // Try to become the initializer
    if region
        .header
        .init_lock
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        reset_region(region);
        region.header.init_lock.store(0, Ordering::Release);
        return;
    }

    // Another process is initializing; spin-wait for it to finish
    for _ in 0..2000 {
        std::thread::yield_now();
        if header_matches(region) {
            return;
        }
    }

    // Timeout: the initializer probably crashed. Force a reset.
    // NOTE: there is a small window where two processes could both
    // reach this point and reset concurrently.  This is acceptable
    // for a best-effort event bus – at worst a few events are lost.
    reset_region(region);
    region.header.init_lock.store(0, Ordering::Release);
}

fn header_matches(region: &SharedRegion) -> bool {
    region.header.magic == SHM_MAGIC
        && region.header.version == SHM_VERSION
        && region.header.max_rings == MAX_RINGS as u32
        && region.header.ring_capacity == RING_CAPACITY as u32
        && region.header.slot_data_size == SLOT_DATA_SIZE as u32
}

fn reset_region(region: &mut SharedRegion) {
    // Zero everything first
    unsafe {
        std::ptr::write_bytes(
            region as *mut SharedRegion as *mut u8,
            0,
            size_of::<SharedRegion>(),
        );
    }

    region.header.magic = SHM_MAGIC;
    region.header.version = SHM_VERSION;
    region.header.max_rings = MAX_RINGS as u32;
    region.header.ring_capacity = RING_CAPACITY as u32;
    region.header.slot_data_size = SLOT_DATA_SIZE as u32;
    region.header.epoch.store(1, Ordering::Relaxed);
    region.header.notify_seq.store(0, Ordering::Relaxed);

    let ring_base =
        size_of::<SharedHeader>() + size_of::<[RingDirectoryEntry; MAX_RINGS]>();
    let ring_bytes = size_of::<PublishRing>() as u32;

    for idx in 0..MAX_RINGS {
        let entry = &region.directory[idx];
        entry.state.store(ENTRY_FREE, Ordering::Relaxed);
        entry.owner_pid.store(0, Ordering::Relaxed);
        entry.generation.store(0, Ordering::Relaxed);
        entry.last_heartbeat_ms.store(0, Ordering::Relaxed);
        entry.ring_offset.store(
            (ring_base + idx * size_of::<PublishRing>()) as u32,
            Ordering::Relaxed,
        );
        entry.ring_bytes.store(ring_bytes, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Ring allocation & activation
// ---------------------------------------------------------------------------

fn allocate_ring(region: &mut SharedRegion, pid: u32) -> Option<usize> {
    // Pass 1: try to grab a FREE entry
    for ring_id in 0..MAX_RINGS {
        let entry = &region.directory[ring_id];
        if entry
            .state
            .compare_exchange(ENTRY_FREE, ENTRY_INIT, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            activate_ring(region, ring_id, pid);
            return Some(ring_id);
        }
    }

    // Pass 2: reclaim entries whose owner is dead
    for ring_id in 0..MAX_RINGS {
        let entry = &region.directory[ring_id];
        let state = entry.state.load(Ordering::Acquire);
        if state != ENTRY_READY && state != ENTRY_INIT {
            continue;
        }

        let owner_pid = entry.owner_pid.load(Ordering::Relaxed);
        if owner_pid != 0 && owner_pid != pid && is_process_alive(owner_pid) {
            continue;
        }

        if entry
            .state
            .compare_exchange(state, ENTRY_INIT, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            activate_ring(region, ring_id, pid);
            return Some(ring_id);
        }
    }

    // Pass 3: check if we already own a READY entry (e.g. after restart
    // with same pid).  IMPORTANT: still call activate_ring to reset the
    // ring contents and bump generation, avoiding stale data.
    for ring_id in 0..MAX_RINGS {
        let entry = &region.directory[ring_id];
        if entry.state.load(Ordering::Acquire) == ENTRY_READY
            && entry.owner_pid.load(Ordering::Relaxed) == pid
        {
            // Transition READY → INIT → activate → READY
            if entry
                .state
                .compare_exchange(
                    ENTRY_READY,
                    ENTRY_INIT,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                activate_ring(region, ring_id, pid);
                return Some(ring_id);
            }
        }
    }

    None
}

fn activate_ring(region: &mut SharedRegion, ring_id: usize, pid: u32) {
    let entry = &region.directory[ring_id];
    let generation = entry.generation.load(Ordering::Relaxed).wrapping_add(1);

    // Reset ring contents
    let ring = &region.rings[ring_id];
    ring.head_seq.store(0, Ordering::Relaxed);
    for slot in &ring.slots {
        slot.seq.store(0, Ordering::Relaxed);
        slot.len.store(0, Ordering::Relaxed);
    }

    // Fill directory entry fields, then publish with release stores
    entry.owner_pid.store(pid, Ordering::Relaxed);
    entry.last_heartbeat_ms.store(now_millis(), Ordering::Relaxed);
    entry.generation.store(generation, Ordering::Release);
    // state = READY is the commit point; consumers only look at READY entries
    entry.state.store(ENTRY_READY, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ringbuffer_path() -> PathBuf {
    std::env::var(DEFAULT_RINGBUFFER_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_RINGBUFFER_PATH))
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid as i32, 0) };
    if rc == 0 {
        return true;
    }
    // EPERM means the process exists but we lack permission to signal it
    let err = std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or_default();
    err == libc::EPERM
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    true
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestEvent {
        id: String,
        seq: u64,
    }

    /// Verify basic single-process publish → drain round-trip.
    /// In the single-process case drain skips our own ring, so we
    /// simulate by directly writing to the ring and reading back.
    #[test]
    fn test_slot_seqlock_roundtrip() {
        // Create a ring in a local buffer (not shared memory)
        let mut ring = unsafe { std::mem::zeroed::<PublishRing>() };

        let payload = b"hello world";
        let seq = 1u64;
        let idx = (seq as usize) & (RING_CAPACITY - 1);
        let slot = &ring.slots[idx];

        // Simulate producer write
        slot.seq.store(0, Ordering::Relaxed);
        unsafe {
            let slot_ptr = slot as *const RingSlot as *mut RingSlot;
            (*slot_ptr).payload[..payload.len()].copy_from_slice(payload);
        }
        slot.len.store(payload.len() as u32, Ordering::Relaxed);
        slot.seq.store(seq, Ordering::Release);
        ring.head_seq.store(seq, Ordering::Release);

        // Simulate consumer read with double-check
        let mut cursor = RingCursor {
            generation: 0,
            read_seq: 0,
        };
        let result = consume_one_payload(&ring, &mut cursor);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), payload.to_vec());
        assert_eq!(cursor.read_seq, 1);
    }

    /// Verify that consumer detects overrun and skips forward.
    #[test]
    fn test_consumer_overrun_skip() {
        let mut ring = unsafe { std::mem::zeroed::<PublishRing>() };

        // Simulate producer having written far ahead
        let head = (RING_CAPACITY as u64) * 3 + 42;
        ring.head_seq.store(head, Ordering::Release);

        // Write the slot that the consumer would try to read after skip
        let oldest_available = head - RING_CAPACITY as u64 + 1;
        let idx = (oldest_available as usize) & (RING_CAPACITY - 1);
        let slot = &ring.slots[idx];
        let payload = b"latest";
        unsafe {
            let slot_ptr = slot as *const RingSlot as *mut RingSlot;
            (*slot_ptr).payload[..payload.len()].copy_from_slice(payload);
        }
        slot.len.store(payload.len() as u32, Ordering::Relaxed);
        slot.seq.store(oldest_available, Ordering::Release);

        let mut cursor = RingCursor {
            generation: 0,
            read_seq: 0, // way behind
        };

        let result = consume_one_payload(&ring, &mut cursor);
        // After overrun detection, cursor jumps forward; first call may
        // or may not return data depending on exact slot state, but
        // cursor.read_seq must have advanced past 0.
        assert!(cursor.read_seq > 0);
        // The cursor should have jumped to at least (head - CAPACITY)
        assert!(cursor.read_seq >= head - RING_CAPACITY as u64);
    }

    /// Verify that header_matches rejects mismatched parameters.
    #[test]
    fn test_header_mismatch() {
        let mut region = unsafe { std::mem::zeroed::<SharedRegion>() };
        assert!(!header_matches(&region));

        region.header.magic = SHM_MAGIC;
        region.header.version = SHM_VERSION;
        region.header.max_rings = MAX_RINGS as u32;
        region.header.ring_capacity = RING_CAPACITY as u32;
        region.header.slot_data_size = SLOT_DATA_SIZE as u32;
        assert!(header_matches(&region));

        region.header.version = 999;
        assert!(!header_matches(&region));
    }
}