/// RangeSet provides a set-like interface for inclusive ranges.
/// Stores sorted, merged ranges and uses binary search for `contains`.
#[derive(Clone, Debug)]
pub struct RangeSet<T> {
    ranges: Vec<std::ops::RangeInclusive<T>>,
}

impl<T: Ord + Copy> From<Vec<std::ops::RangeInclusive<T>>> for RangeSet<T> {
    fn from(mut ranges: Vec<std::ops::RangeInclusive<T>>) -> Self {
        ranges.sort_unstable_by_key(|range| *range.start());
        ranges.dedup_by(|b, a| {
            if b.start() <= a.end() {
                if b.end() > a.end() {
                    *a = *a.start()..=*b.end();
                }
                true
            } else {
                false
            }
        });
        RangeSet { ranges }
    }
}

impl<T: Ord + Copy> FromIterator<T> for RangeSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        Vec::from_iter(items.into_iter().map(|v| v..=v)).into()
    }
}

impl<T> RangeSet<T> {
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
    pub fn contains<Q>(&self, value: &Q) -> bool
    where
        T: std::borrow::Borrow<Q>,
        Q: ?Sized + Ord,
    {
        self.ranges
            .binary_search_by(|range| {
                use std::cmp::Ordering;
                if range.start().borrow() > value {
                    Ordering::Greater
                } else if range.end().borrow() >= value {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            })
            .is_ok()
    }
}
