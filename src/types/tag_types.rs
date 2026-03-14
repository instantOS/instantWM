//! Type-safe tag system.
//!
//! This module provides rich types for tag operations, replacing the primitive `u32` bitmask
//! approach with semantic, type-safe alternatives that improve DX and prevent bugs.

use std::ops::{BitAnd, BitOr, BitXor, Not};
use std::str::FromStr;

use crate::types::{core::SCRATCHPAD_MASK, MAX_TAGS};

/// A type-safe wrapper around tag bitmask operations.
///
/// `TagMask` represents a set of tags as a bitmask, but provides semantic methods
/// for common operations and prevents mixing with arbitrary `u32` values.
///

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct TagMask(u32);

impl TagMask {
    /// Create an empty mask (no tags selected).
    pub const EMPTY: Self = Self(0);

    /// Create a mask with all bits set (all tags selected).
    pub const ALL_BITS: Self = Self(!0);

    /// Mask representing only the scratchpad tag.
    pub const SCRATCHPAD: Self = Self(SCRATCHPAD_MASK);

    /// Create a mask representing a single tag (1-indexed).
    ///
    /// Returns `None` if the tag index is 0 or exceeds `MAX_TAGS`.
    pub fn single(tag_index: usize) -> Option<Self> {
        if tag_index == 0 || tag_index > MAX_TAGS {
            None
        } else {
            Some(Self(1u32 << (tag_index - 1)))
        }
    }

    /// Create a mask from a raw bitmask, validating against max tags.
    ///
    /// Bits beyond `MAX_TAGS` are masked out.
    pub fn from_bits(bits: u32) -> Self {
        let mask = if MAX_TAGS >= 32 {
            bits
        } else {
            bits & ((1u32 << MAX_TAGS) - 1)
        };
        Self(mask)
    }

    /// Create a mask representing all tags up to the given count.
    pub fn all(count: usize) -> Self {
        let count = count.min(MAX_TAGS);
        Self((1u32 << count).wrapping_sub(1))
    }

    /// Get the raw bitmask value.
    pub fn bits(&self) -> u32 {
        self.0
    }

    /// Check if this mask contains a specific tag (1-indexed).
    pub fn contains(&self, tag_index: usize) -> bool {
        if tag_index == 0 || tag_index > MAX_TAGS {
            false
        } else {
            self.0 & (1u32 << (tag_index - 1)) != 0
        }
    }

    /// Check if this mask contains any tags from another mask.
    pub fn intersects(&self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Check if this mask is empty (no tags).
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Check if exactly one tag is selected.
    pub fn is_single(&self) -> bool {
        self.0.count_ones() == 1
    }

    /// Get the index of the lowest set bit (1-indexed), if any.
    pub fn first_tag(&self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some(self.0.trailing_zeros() as usize + 1)
        }
    }

    /// Get the index of the highest set bit (1-indexed), if any.
    pub fn last_tag(&self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some(32 - self.0.leading_zeros() as usize)
        }
    }

    /// Iterate over all set tag indices (1-indexed).
    pub fn iter(&self) -> TagIter {
        TagIter { bits: self.0 }
    }

    /// Count the number of selected tags.
    pub fn count(&self) -> u32 {
        self.0.count_ones()
    }

    /// Returns true if this mask represents only the scratchpad tag.
    ///
    /// Use this to check if a client is on the scratchpad.
    pub fn is_scratchpad_only(self) -> bool {
        self.0 == SCRATCHPAD_MASK
    }

    /// Returns true if this mask contains the scratchpad tag (possibly with other tags).
    pub fn contains_scratchpad(self) -> bool {
        self.0 & SCRATCHPAD_MASK != 0
    }

    /// Returns this mask with the scratchpad bit excluded.
    ///
    /// Use this when computing "occupied tags" for display purposes,
    /// to exclude scratchpad clients.
    pub fn without_scratchpad(self) -> Self {
        Self(self.0 & !SCRATCHPAD_MASK)
    }

    /// Toggle a specific tag in this mask.
    pub fn toggle(&mut self, tag_index: usize) {
        if let Some(mask) = Self::single(tag_index) {
            self.0 ^= mask.0;
        }
    }

    /// Shift all bits left by the given amount (wrapping around max tags).
    pub fn rotate_left(&self, amount: usize, num_tags: usize) -> Self {
        let num_tags = num_tags.min(MAX_TAGS);
        let mask = if num_tags >= 32 {
            !0u32
        } else {
            (1u32 << num_tags) - 1
        };
        let bits = self.0 & mask;
        let amount = amount % num_tags.max(1);
        Self(((bits << amount) | (bits >> (num_tags - amount))) & mask)
    }

    /// Shift all bits right by the given amount (wrapping around max tags).
    pub fn rotate_right(&self, amount: usize, num_tags: usize) -> Self {
        let num_tags = num_tags.min(MAX_TAGS);
        let mask = if num_tags >= 32 {
            !0u32
        } else {
            (1u32 << num_tags) - 1
        };
        let bits = self.0 & mask;
        let amount = amount % num_tags.max(1);
        Self(((bits >> amount) | (bits << (num_tags - amount))) & mask)
    }
}

impl From<TagMask> for u32 {
    fn from(mask: TagMask) -> Self {
        mask.0
    }
}

impl From<&TagMask> for u32 {
    fn from(mask: &TagMask) -> Self {
        mask.0
    }
}

impl BitAnd for TagMask {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitOr for TagMask {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitXor for TagMask {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self::Output {
        Self(self.0 ^ rhs.0)
    }
}

impl Not for TagMask {
    type Output = Self;
    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}

/// Iterator over set tag indices in a mask.
#[derive(Debug, Clone)]
pub struct TagIter {
    bits: u32,
}

impl Iterator for TagIter {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        if self.bits == 0 {
            None
        } else {
            let idx = self.bits.trailing_zeros() as usize + 1;
            self.bits &= self.bits - 1; // Clear lowest set bit
            Some(idx)
        }
    }
}

/// A semantic representation of tag selection intent.
///
/// This enum is used for commands that need to interpret what the user
/// wants to do with tags, rather than just passing raw bitmasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TagSelection {
    /// Select no tags (empty workspace).
    #[default]
    None,
    /// Select a single specific tag.
    Single(usize),
    /// Select multiple specific tags.
    Multi(TagMask),
    /// Select all tags (overview mode).
    All,
    /// Toggle specific tags on/off.
    Toggle(TagMask),
    /// Go to the previously selected tag.
    Previous,
}

impl TagSelection {
    /// Convert this selection to a concrete tag mask.
    ///
    /// # Arguments
    /// * `current_mask` - The current tag mask for context
    /// * `prev_tag` - The previous tag index for Previous variant
    /// * `num_tags` - Total number of available tags
    pub fn to_mask(&self, current_mask: TagMask, prev_tag: usize, num_tags: usize) -> TagMask {
        match *self {
            Self::None => TagMask::EMPTY,
            Self::Single(idx) => TagMask::single(idx).unwrap_or(TagMask::EMPTY),
            Self::Multi(mask) => mask,
            Self::All => TagMask::all(num_tags),
            Self::Toggle(mask) => current_mask ^ mask,
            Self::Previous => TagMask::single(prev_tag).unwrap_or(current_mask),
        }
    }

    /// Check if this selection would result in an empty tag set.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl From<usize> for TagSelection {
    fn from(tag_index: usize) -> Self {
        Self::Single(tag_index)
    }
}

impl From<TagMask> for TagSelection {
    fn from(mask: TagMask) -> Self {
        if mask.is_empty() {
            Self::None
        } else if mask.is_single() {
            Self::Single(mask.first_tag().unwrap_or(0))
        } else {
            Self::Multi(mask)
        }
    }
}

/// A newtype for monitor directions to prevent mixing with other i32 values.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    bincode::Decode,
    bincode::Encode,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct MonitorDirection(pub i32);

impl MonitorDirection {
    /// Move to the next monitor (right/down).
    pub const NEXT: Self = Self(1);
    /// Move to the previous monitor (left/up).
    pub const PREV: Self = Self(-1);

    /// Create a direction from an arbitrary value.
    pub fn new(value: i32) -> Self {
        Self(value.signum())
    }

    /// Get the raw direction value.
    pub fn value(&self) -> i32 {
        self.0
    }

    /// Check if this is a "next" direction.
    pub fn is_next(&self) -> bool {
        self.0 > 0
    }

    /// Check if this is a "previous" direction.
    pub fn is_prev(&self) -> bool {
        self.0 < 0
    }
}

impl Default for MonitorDirection {
    fn default() -> Self {
        Self::NEXT
    }
}

impl FromStr for MonitorDirection {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "next" | "right" | "down" | "1" => Ok(Self::NEXT),
            "prev" | "previous" | "left" | "up" | "-1" => Ok(Self::PREV),
            _ => Err(()),
        }
    }
}

impl clap::ValueEnum for MonitorDirection {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::NEXT, Self::PREV]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self.0 {
            1 => Some(clap::builder::PossibleValue::new("next")),
            -1 => Some(clap::builder::PossibleValue::new("prev")),
            _ => None,
        }
    }

    fn from_str(s: &str, _ignore_case: bool) -> Result<Self, String> {
        FromStr::from_str(s).map_err(|_| format!("Invalid direction: {}", s))
    }
}

impl From<i32> for MonitorDirection {
    fn from(value: i32) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_mask_single() {
        assert_eq!(TagMask::single(0), None);
        assert_eq!(TagMask::single(1).map(|m| m.bits()), Some(1));
        assert_eq!(TagMask::single(3).map(|m| m.bits()), Some(4));
        assert!(TagMask::single(MAX_TAGS + 1).is_none());
    }

    #[test]
    fn test_tag_mask_contains() {
        let mask = TagMask::from_bits(0b1010); // Tags 2 and 4
        assert!(!mask.contains(1));
        assert!(mask.contains(2));
        assert!(!mask.contains(3));
        assert!(mask.contains(4));
    }

    #[test]
    fn test_tag_mask_iter() {
        let mask = TagMask::from_bits(0b1010);
        let tags: Vec<_> = mask.iter().collect();
        assert_eq!(tags, vec![2, 4]);
    }

    #[test]
    fn test_tag_mask_operations() {
        let a = TagMask::from_bits(0b1010);
        let b = TagMask::from_bits(0b1100);

        assert_eq!((a & b).bits(), 0b1000);
        assert_eq!((a | b).bits(), 0b1110);
        assert_eq!((a ^ b).bits(), 0b0110);
    }

    #[test]
    fn test_tag_selection_to_mask() {
        let current = TagMask::from_bits(0b0001);

        assert_eq!(TagSelection::None.to_mask(current, 2, 9).bits(), 0);
        assert_eq!(
            TagSelection::Single(3).to_mask(current, 2, 9).bits(),
            0b0100
        );
        assert_eq!(TagSelection::All.to_mask(current, 2, 4).bits(), 0b1111);
    }
}
