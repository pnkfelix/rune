#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_panics_doc)]
use crate::metric::{BufferMetrics, Metric};
use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    ops::{Bound, Deref, Range, RangeBounds},
};
use str_indices::chars;

/// A Gap buffer. This represents the text of a buffer, and allows for
/// efficient insertion and deletion of text.
#[derive(Default)]
pub struct Buffer {
    /// The buffer data
    data: Box<[u8]>,
    /// start of the gap. Both gap_start and gap_end are the same point, but
    /// gap_start is never a valid byte index, and gap_end is always used
    /// instead.
    gap_start: usize,
    /// The end of the gap in bytes
    gap_end: usize,
    /// The number of characters until the gap
    gap_chars: usize,
    /// The current cursor.
    cursor: Metric,
    total: Metric,
    metrics: BufferMetrics,
}

impl Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let start = self.to_str(..self.gap_start);
        let end = self.to_str(self.gap_end..);
        // repeat _ for the gap length
        let gap = "_".repeat(self.gap_len());
        f.debug_struct("Buffer")
            .field("data", &format!("{start}{gap}{end}"))
            .field("gap_start", &self.gap_start)
            .field("gap_end", &self.gap_end)
            .field("gap_chars", &self.gap_chars)
            .field("cursor", &self.cursor)
            .field("metrics", &self.metrics)
            .field("total_chars", &self.total.chars)
            .finish()
    }
}

impl Display for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.read(0..self.len()))
    }
}

const METRIC_SIZE: usize = crate::metric::MAX_LEAF / 2;
struct MetricBuilder<'a> {
    slice: &'a str,
    start: usize,
    end: usize,
}

impl<'a> MetricBuilder<'a> {
    fn new(slice: &'a str) -> Self {
        Self { slice, start: 0, end: slice.len().min(METRIC_SIZE) }
    }
}

impl<'a> Iterator for MetricBuilder<'a> {
    type Item = Metric;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.slice.len() {
            return None;
        }
        let mut end = self.end;
        if end != self.slice.len() {
            while !self.slice.is_char_boundary(end) {
                end -= 1;
            }
        }
        let slice = &self.slice[self.start..end];
        self.start = end;
        self.end += METRIC_SIZE;
        Some(metrics(slice))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.slice.len() - self.start;
        let extra = usize::from(len % METRIC_SIZE != 0);
        let size = len / METRIC_SIZE;
        (size + extra, None)
    }
}

impl From<&str> for Buffer {
    fn from(data: &str) -> Self {
        let storage = {
            let capacity = data.len() + Self::GAP_SIZE;
            let mut storage = Vec::with_capacity(capacity);
            storage.resize(Self::GAP_SIZE, 0);
            storage.extend_from_slice(data.as_bytes());
            assert_eq!(storage.len(), capacity);
            storage.into_boxed_slice()
        };
        let builder = MetricBuilder::new(data);
        let metrics = BufferMetrics::build(builder);
        Self {
            data: storage,
            gap_start: 0,
            gap_end: Self::GAP_SIZE,
            gap_chars: 0,
            cursor: Metric { bytes: Self::GAP_SIZE, chars: 0 },
            total: metrics.len(),
            metrics,
        }
    }
}

impl<T> PartialEq<T> for Buffer
where
    T: Deref<Target = str>,
{
    fn eq(&self, other: &T) -> bool {
        PartialEq::eq(self, Deref::deref(other))
    }
}

impl PartialEq<str> for Buffer {
    fn eq(&self, other: &str) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.to_str(..self.gap_start) == &other[..self.gap_start]
            && self.to_str(self.gap_end..) == &other[self.gap_start..]
    }
}

impl Buffer {
    #[cfg(not(test))]
    const GAP_SIZE: usize = 2000;
    #[cfg(test)]
    const GAP_SIZE: usize = 5;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn grow(&mut self, slice: &str) {
        let new_capacity = {
            let pre_gap = self.gap_start;
            let post_gap = self.data.len() - self.gap_end;
            pre_gap + slice.len() + Self::GAP_SIZE + post_gap
        };
        let new_storage = {
            let mut buffer = Vec::with_capacity(new_capacity);
            // pre-gap
            buffer.extend_from_slice(&self.data[..self.gap_start]);
            // new text
            buffer.extend_from_slice(slice.as_bytes());
            // gap
            buffer.resize(buffer.len() + Self::GAP_SIZE, 0);
            // post-gap
            buffer.extend_from_slice(&self.data[self.gap_end..]);
            buffer.into_boxed_slice()
        };
        assert_eq!(new_storage.len(), new_capacity);
        self.data = new_storage;
        self.gap_start += slice.len();
        self.gap_end = self.gap_start + Self::GAP_SIZE;
        self.cursor.bytes = self.gap_end;
        let new = metrics(slice);
        self.gap_chars += new.chars;
        self.cursor.chars = self.gap_chars;
        self.total += new;
    }

    pub fn insert_char(&mut self, chr: char) {
        let buf = &mut [0; 4];
        self.insert(chr.encode_utf8(buf));
    }

    pub fn insert(&mut self, slice: &str) {
        self.metrics.insert(self.to_abs_pos(self.cursor), MetricBuilder::new(slice));
        // if gap is not at cursor, move it there
        if self.gap_chars != self.cursor.chars {
            // TODO: we don't need to recalculate the position
            self.move_gap(self.cursor);
        }
        if self.gap_len() < slice.len() {
            // TODO: grow the gap and move the cursor in one go
            self.grow(slice);
        } else {
            let new_slice = &mut self.data[self.gap_start..(self.gap_start + slice.len())];
            new_slice.copy_from_slice(slice.as_bytes());
            self.gap_start += slice.len();
            let new = metrics(slice);
            self.gap_chars += new.chars;
            self.cursor.chars += new.chars;
            self.total += new;
        }
    }

    pub fn delete_backwards(&mut self, size: usize) {
        let size = size.min(self.cursor.chars);
        self.delete_range(self.cursor.chars - size, self.cursor.chars);
    }

    pub fn delete_forwards(&mut self, size: usize) {
        self.delete_range(self.cursor.chars, self.cursor.chars + size);
    }

    pub fn delete_range(&mut self, beg: usize, end: usize) {
        let (mut beg_chars, mut end_chars) = (beg, end);
        if beg_chars > end_chars {
            (beg_chars, end_chars) = (end_chars, beg_chars);
        }
        end_chars = end_chars.min(self.total.chars);
        beg_chars = beg_chars.min(self.total.chars);
        let end_bytes = self.char_to_byte(end_chars);
        let beg_bytes = self.char_to_byte(beg_chars);
        if end_bytes != beg_bytes {
            let beg = Metric { bytes: beg_bytes, chars: beg_chars };
            let end = Metric { bytes: end_bytes, chars: end_chars };
            self.metrics.delete(self.to_abs_pos(beg), self.to_abs_pos(end));
            self.delete_byte_range(beg, end);
        }
    }

    fn delete_byte_range(&mut self, beg: Metric, end: Metric) {
        // TODO: optimize this so that we count the chars deleted when calculating position
        assert!(beg.bytes <= end.bytes, "beg ({beg}) is greater then end ({end})");
        assert!(end.bytes <= self.data.len(), "end out of bounds");
        self.assert_char_boundary(beg.bytes);
        self.assert_char_boundary(end.bytes);
        if end.bytes < self.gap_start {
            // delete before gap
            //
            // hello New York City||||||||||
            //     ^      ^       ^         ^
            //     beg    end     gap_start gap_end
            //
            // shift end..gap_start to the right
            //
            // hell|||||||||||||||||ork City
            //     ^                ^
            //     gap_start        gap_end

            // update character count
            let deleted = end - beg;
            let delete_offset_chars = self.gap_chars - end.chars;
            self.gap_chars -= deleted.chars + delete_offset_chars;
            self.total -= deleted;
            let new_end = self.gap_end - (self.gap_start - end.bytes);
            // shift data
            self.data.copy_within(end.bytes..self.gap_start, new_end);
            // update cursor
            self.update_cursor_chars(beg.bytes, end.bytes, deleted.chars);
            if self.cursor.bytes < self.gap_start {
                if self.cursor.bytes > end.bytes {
                    self.cursor.bytes += self.gap_len();
                } else if self.cursor.bytes >= beg.bytes {
                    self.cursor.bytes = new_end;
                }
            }
            // update gap position
            self.gap_end = new_end;
            self.gap_start = beg.bytes;
        } else if beg.bytes >= self.gap_end {
            // delete after gap
            //
            // ||||||||||hello New York City
            // ^         ^         ^   ^
            // gap_start gap_end   beg end
            //
            // shift gap_end..beg to the left
            //
            // hello New |||||||||||||| City
            //           ^             ^
            //           gap_start     gap_end

            // update character count

            let deleted = end - beg;
            self.total -= deleted;
            self.gap_chars += beg.chars - self.gap_chars;
            // shift data
            self.data.copy_within(self.gap_end..beg.bytes, self.gap_start);
            // update cursor
            self.update_cursor_chars(beg.bytes, end.bytes, deleted.chars);
            if self.cursor.bytes >= self.gap_end {
                if self.cursor.bytes < beg.bytes {
                    self.cursor.bytes -= self.gap_len();
                } else if self.cursor.bytes < end.bytes {
                    self.cursor.bytes = end.bytes;
                }
            }
            // update gap position
            self.gap_start += beg.bytes - self.gap_end;
            self.gap_end = end.bytes;
        } else if beg.bytes < self.gap_start && end.bytes >= self.gap_end {
            // delete spans gap
            //
            // hello|||||||||| New York City
            //  ^   ^         ^       ^
            //  beg gap_start gap_end end
            //
            // update start and end of gap
            //
            // h||||||||||||||||||||||k City
            //  ^                     ^
            //  gap_start             gap_end

            // update character count
            let gap_start = Metric { bytes: self.gap_start, chars: self.gap_chars };
            let before = gap_start - beg;
            let gap_end = Metric { bytes: self.gap_end, chars: self.gap_chars };
            let after = end - gap_end;
            self.gap_chars -= before.chars;
            self.total -= before + after;
            // update gap position
            self.gap_start = beg.bytes;
            self.gap_end = end.bytes;
            self.update_cursor_chars(beg.bytes, end.bytes, before.chars + after.chars);
            if (beg.bytes..end.bytes).contains(&self.cursor.bytes) {
                self.cursor.bytes = end.bytes;
            }
        } else {
            panic!(
                "delete region inside gap -- gap: {}-{}, span: {beg}-{end}",
                self.gap_start, self.gap_end
            );
        }
    }

    fn update_cursor_chars(&mut self, beg: usize, end: usize, size: usize) {
        if self.cursor.bytes > beg {
            if self.cursor.bytes > end {
                self.cursor.chars -= size;
            } else {
                self.cursor.chars = self.gap_chars;
            }
        }
    }

    pub fn move_gap_out_of(&mut self, range: impl RangeBounds<usize>) {
        if !range.contains(&self.gap_chars)
            || range.start_bound() == Bound::Included(&self.gap_chars)
        {
            return;
        }

        let start = match range.start_bound() {
            Bound::Included(x) => *x,
            Bound::Excluded(_) => unreachable!(),
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(_) => unimplemented!("inclusive end bound not supported"),
            Bound::Excluded(x) => *x,
            Bound::Unbounded => self.total.chars,
        };

        let pos = if self.gap_chars - start < end - self.gap_chars {
            Metric { bytes: self.char_to_byte(start), chars: start }
        } else {
            Metric { bytes: self.char_to_byte(end), chars: end }
        };
        self.move_gap(pos);
    }

    fn move_gap(&mut self, pos: Metric) {
        assert!(pos.bytes <= self.data.len(), "attempt to move gap out of bounds");
        self.assert_char_boundary(pos.bytes);
        if pos.bytes < self.gap_start {
            // move gap backwards
            let shift = Metric { bytes: self.gap_start, chars: self.gap_chars } - pos;
            self.gap_chars -= shift.chars;

            self.data.copy_within(pos.bytes..self.gap_start, self.gap_end - shift.bytes);
            // if gap moves across cursor, update cursor position
            if self.cursor.bytes < self.gap_start && self.cursor.bytes >= pos.bytes {
                self.cursor.bytes += self.gap_len();
            }
            self.gap_start = pos.bytes;
            self.gap_end -= shift.bytes;
        } else if pos.bytes >= self.gap_end {
            // move gap forwards
            self.gap_chars += pos.chars - self.gap_chars;
            self.data.copy_within(self.gap_end..pos.bytes, self.gap_start);
            let size = pos.bytes - self.gap_end;
            // if gap moves across cursor, update cursor position
            if self.cursor.bytes >= self.gap_end && self.cursor.bytes < pos.bytes {
                self.cursor.bytes -= self.gap_len();
            }
            self.gap_start += size;
            self.gap_end = pos.bytes;
        } else {
            panic!(
                "move gap position byte: ({pos}) inside gap ({}-{})",
                self.gap_start, self.gap_end
            );
        }
    }

    pub fn set_cursor(&mut self, pos: usize) {
        let pos = pos.min(self.total.chars);
        let byte_pos = self.char_to_byte(pos);
        self.cursor = Metric { bytes: byte_pos, chars: pos };
    }

    fn to_abs_pos(&self, pos: Metric) -> Metric {
        let chars = pos.chars;
        let bytes = if pos.bytes < self.gap_start {
            pos.bytes
        } else if pos.bytes >= self.gap_end {
            pos.bytes - self.gap_len()
        } else {
            unreachable!()
        };
        Metric { bytes, chars }
    }

    fn to_gapped_pos(&self, pos: Metric) -> Metric {
        let chars = pos.chars;
        let bytes = if pos.bytes < self.gap_start {
            pos.bytes
        } else if pos.bytes >= self.gap_start {
            pos.bytes + self.gap_len()
        } else {
            unreachable!()
        };
        Metric { bytes, chars }
    }

    pub fn len(&self) -> usize {
        debug_assert_eq!(self.total.bytes + self.gap_len(), self.data.len());
        self.total.bytes
    }

    pub const fn len_chars(&self) -> usize {
        self.total.chars
    }

    pub const fn is_empty(&self) -> bool {
        self.total.chars == 0
    }

    const fn gap_len(&self) -> usize {
        self.gap_end - self.gap_start
    }

    fn char_to_byte(&self, pos: usize) -> usize {
        if pos == 0 {
            return if self.gap_start == 0 { self.gap_end } else { 0 };
        }
        if pos == self.total.chars {
            return self.data.len();
        }
        if pos == self.gap_chars {
            return self.gap_end;
        }
        let (base, offset) = self.metrics.search_char(pos);
        debug_assert_eq!(base.chars + offset, pos);

        let base = self.to_gapped_pos(base);

        if offset == 0 {
            return base.bytes;
        }

        self.assert_char_boundary(base.bytes);

        if base.chars < self.gap_chars {
            if pos < self.gap_chars {
                let string = self.to_str(base.bytes..self.gap_start);
                chars::to_byte_idx(string, offset) + base.bytes
            } else {
                // the char crosses the gap
                let string = self.to_str(self.gap_end..);
                self.gap_end + chars::to_byte_idx(string, pos - self.gap_chars)
            }
        } else {
            let string = self.to_str(base.bytes..);
            chars::to_byte_idx(string, offset) + base.bytes
        }
    }

    fn to_str(&self, range: impl std::slice::SliceIndex<[u8], Output = [u8]>) -> &str {
        // TODO: remove this check once we are confident the code is correct
        std::str::from_utf8(&self.data[range]).unwrap()
    }

    pub fn read(&self, byte_range: Range<usize>) -> Cow<'_, str> {
        // if past gap_start, add gap_len to range
        let mut range = byte_range.clone();
        if range.start >= self.gap_start {
            range.start += self.gap_len();
        }
        if range.end >= self.gap_start {
            range.end += self.gap_len();
        }
        assert!(range.end <= self.data.len(), "range end out of bounds");
        assert!(range.start <= self.data.len(), "range start out of bounds");
        for i in 0..4 {
            if self.is_char_boundary(range.end - i) {
                range.end -= i;
                break;
            }
        }
        for i in 0..4 {
            if self.is_char_boundary(range.start + i) {
                range.start += i;
                break;
            }
        }
        // assert the range does not overlap with the gap
        assert!(range.start >= self.gap_end || range.start < self.gap_start);
        assert!(range.end >= self.gap_end || range.end < self.gap_start);

        // the range straddles the gap, so we need to copy the two halves
        if range.start < self.gap_start && self.gap_start < range.end {
            let mut string = String::with_capacity(range.len());
            string.push_str(self.to_str(range.start..self.gap_start));
            string.push_str(self.to_str(self.gap_end..range.end));
            assert_eq!(string.len(), byte_range.len());
            Cow::Owned(string)
        } else {
            Cow::Borrowed(self.to_str(range))
        }
    }

    fn assert_char_boundary(&self, pos: usize) {
        if cfg!(debug_assertions) {
            if pos == self.gap_start {
                return;
            }
            assert!(self.is_char_boundary(pos), "position ({pos}) not on utf8 boundary");
        }
    }

    fn is_char_boundary(&self, pos: usize) -> bool {
        match self.data.get(pos) {
            Some(byte) => is_char_boundary(*byte),
            None => pos == self.data.len(),
        }
    }
}

fn metrics(slice: &str) -> Metric {
    let chars = chars::count(slice);
    Metric { bytes: slice.len(), chars }
}

#[allow(clippy::cast_possible_wrap)]
const fn is_char_boundary(byte: u8) -> bool {
    // This is bit magic equivalent to: b < 128 || b >= 192
    (byte as i8) >= -0x40
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create() {
        let string = "hello buffer";
        let buffer = Buffer::from(string);
        assert_eq!(buffer.data.len(), string.len() + Buffer::GAP_SIZE);
        assert_eq!(buffer.gap_end, Buffer::GAP_SIZE);
        assert_eq!(buffer.gap_start, 0);
    }

    #[test]
    fn test_empty() {
        let mut buffer = Buffer::new();
        assert_eq!(buffer.data.len(), 0);
        assert_eq!(buffer.gap_len(), 0);
        assert_eq!(buffer, "");
        buffer.insert("hello");
        assert_eq!(buffer, "hello");

        let mut buffer = Buffer::new();
        buffer.delete_range(0, 0);
        assert_eq!(buffer, "");
        buffer.delete_range(0, 5);
        assert_eq!(buffer, "");
    }

    #[test]
    fn insert() {
        let string = "hello buffer";
        let mut buffer = Buffer::from(string);
        println!("insert");
        buffer.len();
        buffer.insert_char('x');
        println!("insert");
        buffer.len();
        println!("insert");
        assert_eq!(buffer.data.len(), string.len() + Buffer::GAP_SIZE);
        assert_eq!(buffer.gap_end, Buffer::GAP_SIZE);
        assert_eq!(buffer.gap_start, 1);
        assert_eq!(buffer, "xhello buffer");
    }

    #[test]
    fn insert_slice() {
        let string = "world";
        let new_string = "hi ";
        let mut buffer = Buffer::from(string);
        buffer.insert(new_string);
        buffer.move_gap_out_of(..);
        assert_eq!(buffer, "hi world");
        buffer.insert("starting Θ text ");
        assert_eq!(buffer, "hi starting Θ text world");
        buffer.set_cursor(21);
        buffer.insert("x");
        assert_eq!(buffer, "hi starting Θ text woxrld");
    }

    #[test]
    fn empty() {
        let mut buffer = Buffer::from("");
        assert_eq!(buffer, "");
        buffer.delete_range(1, 2);
        assert_eq!(buffer, "");
    }

    #[test]
    fn test_delete() {
        let world = "world";
        let hello = "hello ";
        let mut buffer = Buffer::from(world);
        buffer.insert(hello);
        buffer.delete_backwards(4);
        assert_eq!(buffer.gap_start, hello.len() - 4);
        assert_eq!(buffer.gap_end, hello.len() + Buffer::GAP_SIZE);
        buffer.move_gap_out_of(..);
        buffer.move_gap_out_of(..);
        buffer.move_gap(Metric { bytes: buffer.char_to_byte(7), chars: 7 });
        buffer.move_gap_out_of(..);
        assert_eq!(buffer, "heworld");
    }

    #[test]
    fn delete_forwards() {
        let world = "world";
        let hello = "hello ";
        let mut buffer = Buffer::from(world);
        buffer.insert(hello);
        buffer.delete_forwards(4);
        buffer.move_gap_out_of(..);
        assert_eq!(buffer, "hello d");
    }

    #[test]
    fn test_delete_region() {
        let mut buffer = Buffer::from("world");
        buffer.insert("hello ");
        buffer.delete_range(1, 3);
        buffer.move_gap_out_of(..);
        assert_eq!(buffer, "hlo world");
        buffer.delete_range(4, 6);
        buffer.move_gap_out_of(..);
        assert_eq!(buffer, "hlo rld");
    }

    #[test]
    fn test_delete_nothing() {
        let mut buffer = Buffer::from("world");
        buffer.insert("hello ");
        buffer.delete_range(3, 3);
        assert_eq!(buffer, "hello world");
    }

    // cases found during fuzzing
    #[test]
    fn edge_cases() {
        let mut buffer = Buffer::from(":?abdix7");
        assert_eq!(buffer.len(), 8);
        buffer.delete_range(2, 5);
        assert_eq!(buffer.len(), 5);
        buffer.delete_range(5, 4);
        assert_eq!(buffer.len(), 4);
        buffer.delete_range(0, 3);

        let mut buffer = Buffer::from("xyz");
        buffer.insert("abc");
        buffer.set_cursor(2);
        buffer.delete_range(1, 4);
        assert_eq!(buffer, "ayz");
        buffer.insert("b");
        assert_eq!(buffer, "abyz");

        let mut buffer = Buffer::from("ƽaejcoeuz");
        buffer.delete_range(5, 6);
        buffer.delete_range(1, 8);
        assert_eq!(buffer, "ƽ");
    }

    // from reference implementation
    #[test]
    fn test_delete_to_gap() {
        let mut buffer = Buffer::from("\n\n\n\nAutomerge is too");
        buffer.insert("per. Some graduate students in ");
        buffer.set_cursor(10);
        buffer.delete_forwards(21);
        assert_eq!(buffer, "per. Some \n\n\n\nAutomerge is too");
    }

    // fuzzing
    #[test]
    fn test_bounds() {
        let mut buffer = Buffer::from("world");
        buffer.insert("hello ");
        buffer.delete_range(3, 100);
        assert_eq!(buffer, "hel");
        buffer.delete_range(10, 1);
        assert_eq!(buffer, "h");

        let mut buffer = Buffer::from(",skeobg x");
        buffer.delete_range(10, 10);
        assert_eq!(buffer.gap_len(), 5);

        let mut buffer = Buffer::from("+skeocptv'eigp");
        buffer.delete_range(30, 6);
        assert_eq!(buffer.gap_len(), 13);
    }

    #[test]
    fn resize() {
        let world = "world";
        let hello = "hello ";
        let mut buffer = Buffer::from(world);
        buffer.insert(hello);
        assert_eq!(buffer.data.len(), hello.len() + world.len() + Buffer::GAP_SIZE);
        assert_eq!(buffer.gap_end, hello.len() + Buffer::GAP_SIZE);
        assert_eq!(buffer.gap_start, hello.len());
        assert_eq!(buffer, "hello world");
    }

    #[test]
    fn cursor() {
        let string = "world";
        let new_string = "hi ";
        let mut buffer = Buffer::from(string);
        buffer.insert(new_string);
        assert_eq!(buffer.gap_chars, new_string.len());
    }

    #[test]
    fn test_read() {
        let mut buffer = Buffer::from("hello world");
        buffer.set_cursor(5);
        assert_eq!(buffer.read(0..0), Cow::Borrowed(""));
        assert_eq!(buffer.read(0..5), Cow::Borrowed("hello"));
        assert_eq!(buffer.read(5..11), Cow::Borrowed(" world"));
        assert_eq!(buffer.read(4..6), Cow::<str>::Owned(String::from("o ")));
    }

    #[test]
    fn test_build_unicode() {
        let string = "aaaaaaaaaՂaaaaaaaaa";
        let _ = Buffer::from(string);
    }

    #[test]
    fn test_append() {
        let mut buffer = Buffer::from("aa");
        buffer.set_cursor(3);
        let string = "B\u{1b}BBBBBB\u{1b}\0\0\0\0\0\0BB\u{1b}\u{1b}\u{1b}\u{1b}\u{1b}\u{1b}B\u{7}BBBBBBBBBBB\u{1b}\u{1b}\u{1b}B\u{7}BBBBBBBBBBBB\u{1b}\u{1b}B\u{7}BBBBBBBBB";
        buffer.insert(string);
    }

    #[test]
    fn test_fuzzer() {
        let mut buffer = Buffer::new();
        buffer.set_cursor(1);
        buffer.insert("Ղ\u{2}\u{2}\0\0\0");
        buffer.set_cursor(4);
        buffer.insert("&\0''''''''''''''''''''%'''''&\0''''''''''''''''''''%@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@''''''''''");
        buffer.set_cursor(39);
        buffer.insert("'\u{2}&\0''''''''''''''''''''%''''''''''''''''''''''''''''");
        buffer.delete_range(184, 169);
        buffer.set_cursor(127);
        buffer.insert("00000000061288823:*********");
        buffer.set_cursor(132);
        buffer.insert("5''''''''''''''\0\0\0\0\0'''''''");
        buffer.set_cursor(97);
        buffer.insert("''?????????????????????z?????????????????????'''''\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{1d}\u{10}");
        buffer.delete_range(13, 138);
        buffer.set_cursor(25);
        buffer
            .insert("yyyyyyyyyyyyyy\u{2}\0\u{2}\0\0\u{1}\u{17}H\u{17}\u{17}\u{17}\u{17}\u{17}\0\0");
        buffer.set_cursor(138);
        buffer.insert("\u{17}?\u{17}\u{17}\u{17}\u{17}\u{17}\u{17}\u{17}\u{17}\u{17}\0\0\0\0\0\0\u{3}\0\0\0''''''''");
        buffer.set_cursor(39);
        buffer.insert("\0\0\0''''''''''");
        buffer.delete_range(247, 45);
    }

    #[test]
    fn test_pos() {
        let mut buffer = Buffer::new();
        buffer.set_cursor(1);
        buffer.insert("AAAAAAAAAAAAAAAAAAA");
        buffer.set_cursor(10);
        buffer.insert("AAAAAA\0\0AAAAAA");
        buffer.set_cursor(26);
    }
}
