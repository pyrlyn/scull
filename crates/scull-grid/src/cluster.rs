//! Interned grapheme clusters: the text of cells holding more than one code
//! point. Separate because a single code point lives inline in the cell
//! (foot's design, `docs/research/foot-contour.md` §1.6), so only the rare
//! multi-code-point cluster pays for a table entry.

use std::sync::Arc;

use crate::GridError;
use crate::intern::{Interner, Marks};

/// Live clusters at most. Each holds at most [`MAX_CLUSTER_CHARS`] code
/// points, so a full table stays under a few MiB.
pub const MAX_CLUSTERS: usize = 1 << 16;

/// Code points one cell may hold. Stream-safe text (UAX #15) never needs more
/// than 31; kitty caps at 24 and Contour at 16, which cut real ZWJ emoji
/// with skin tones and tag sequences less often than this does.
pub const MAX_CLUSTER_CHARS: usize = 32;

/// Index of a cluster in a [`ClusterTable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClusterId(pub u32);

/// Distinct multi-code-point clusters, capped at [`MAX_CLUSTERS`].
#[derive(Debug, Clone)]
pub struct ClusterTable(Interner<Arc<str>>);

impl Default for ClusterTable {
    fn default() -> Self {
        Self::with_cap(MAX_CLUSTERS)
    }
}

impl ClusterTable {
    /// A table holding at most `cap` live clusters (clamped to [`MAX_CLUSTERS`]).
    pub fn with_cap(cap: usize) -> Self {
        Self(Interner::new(cap.min(MAX_CLUSTERS), []))
    }

    /// The id of `text`, adding it when new. `text` must hold two to
    /// [`MAX_CLUSTER_CHARS`] code points.
    pub fn intern(&mut self, text: &str) -> Result<ClusterId, GridError> {
        let len = text.chars().take(MAX_CLUSTER_CHARS + 1).count();
        if len < 2 {
            return Err(GridError::ClusterTooShort);
        }
        if len > MAX_CLUSTER_CHARS {
            return Err(GridError::ClusterTooLong {
                len: text.chars().count(),
                max: MAX_CLUSTER_CHARS,
            });
        }
        self.0
            .intern(text, |t| Arc::from(t))
            .map(ClusterId)
            .ok_or(GridError::ClusterTableFull { cap: self.0.cap() })
    }

    /// The text behind `id`; `None` for an id that was reclaimed.
    pub fn get(&self, id: ClusterId) -> Option<&str> {
        self.0.get(id.0).map(|t| &**t)
    }

    /// Live clusters.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no cluster is live.
    pub fn is_empty(&self) -> bool {
        self.0.len() == 0
    }

    /// The hard cap of live clusters.
    pub fn cap(&self) -> usize {
        self.0.cap()
    }

    /// Clusters added since the last sweep; the grid sweeps only
    /// when enough were added for the walk over every row to pay off.
    pub fn inserts_since_sweep(&self) -> usize {
        self.0.inserts_since_sweep()
    }

    /// A blank mark set for [`Self::sweep`].
    pub fn marks(&self) -> Marks {
        self.0.marks()
    }

    /// Reclaims every id `live` does not mark; returns how many. Ids held
    /// outside the marked rows are invalid afterwards. Marks made before
    /// the latest intern reclaim nothing.
    pub fn sweep(&mut self, live: &Marks) -> usize {
        self.0.sweep(live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

    #[test]
    fn cluster_round_trips_through_its_id() {
        let mut t = ClusterTable::default();
        let id = t.intern(FAMILY).unwrap();
        assert_eq!(t.get(id), Some(FAMILY));
        assert_eq!(t.intern(FAMILY), Ok(id));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn single_code_point_is_not_a_cluster() {
        let mut t = ClusterTable::default();
        assert_eq!(t.intern("a"), Err(GridError::ClusterTooShort));
        assert_eq!(t.intern(""), Err(GridError::ClusterTooShort));
    }

    #[test]
    fn cluster_longer_than_the_cap_is_refused() {
        let mut t = ClusterTable::default();
        let at_cap: String = std::iter::once('e')
            .chain(std::iter::repeat_n('\u{301}', MAX_CLUSTER_CHARS - 1))
            .collect();
        assert!(t.intern(&at_cap).is_ok());
        let over = format!("{at_cap}\u{301}");
        assert_eq!(
            t.intern(&over),
            Err(GridError::ClusterTooLong {
                len: MAX_CLUSTER_CHARS + 1,
                max: MAX_CLUSTER_CHARS
            })
        );
    }

    #[test]
    fn cluster_table_refuses_clusters_past_its_cap() {
        const CAP: usize = 2;
        let mut t = ClusterTable::with_cap(CAP);
        assert!(t.intern("a\u{301}").is_ok());
        assert!(t.intern("e\u{301}").is_ok());
        assert_eq!(
            t.intern("o\u{301}"),
            Err(GridError::ClusterTableFull { cap: CAP })
        );
    }

    #[test]
    fn requested_cap_is_clamped_to_the_hard_cap() {
        let t = ClusterTable::with_cap(usize::MAX);
        assert_eq!(t.cap(), MAX_CLUSTERS);
    }
}
