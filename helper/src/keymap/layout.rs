//! Physical layout of the OP36: 36 keys, matrix (row, col) per key, hand, column and thumb info.
//!
//! Index order is reading order on the vendor's pictures: three rows of ten (left pinky → right
//! pinky), then the six thumb keys (left outer → inner, right inner → outer). Matrix positions come
//! from `matrix_map` in the vendor's `keyboards/op36/keyboard.toml`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalKey {
    pub index: u8,
    pub row: u8,
    pub col: u8,
    pub hand: Hand,
    /// 0 = pinky … 4 = inner column; `None` for thumbs.
    pub column: Option<u8>,
    /// 0 = outer … 2 = inner thumb key; `None` for the main grid.
    pub thumb: Option<u8>,
    pub home_row: bool,
}

const fn grid(index: u8, row: u8, col: u8, hand: Hand, column: u8, home_row: bool) -> PhysicalKey {
    PhysicalKey {
        index,
        row,
        col,
        hand,
        column: Some(column),
        thumb: None,
        home_row,
    }
}

const fn thumb(index: u8, row: u8, col: u8, hand: Hand, thumb: u8) -> PhysicalKey {
    PhysicalKey {
        index,
        row,
        col,
        hand,
        column: None,
        thumb: Some(thumb),
        home_row: false,
    }
}

pub const OP36_KEYS: [PhysicalKey; 36] = {
    use Hand::{Left as L, Right as R};
    [
        // row 0
        grid(0, 0, 4, L, 0, false),
        grid(1, 0, 3, L, 1, false),
        grid(2, 0, 2, L, 2, false),
        grid(3, 0, 1, L, 3, false),
        grid(4, 0, 0, L, 4, false),
        grid(5, 3, 0, R, 4, false),
        grid(6, 3, 1, R, 3, false),
        grid(7, 3, 2, R, 2, false),
        grid(8, 3, 3, R, 1, false),
        grid(9, 3, 4, R, 0, false),
        // row 1 (home row)
        grid(10, 1, 4, L, 0, true),
        grid(11, 1, 3, L, 1, true),
        grid(12, 1, 2, L, 2, true),
        grid(13, 1, 1, L, 3, true),
        grid(14, 1, 0, L, 4, false),
        grid(15, 4, 0, R, 4, false),
        grid(16, 4, 1, R, 3, true),
        grid(17, 4, 2, R, 2, true),
        grid(18, 4, 3, R, 1, true),
        grid(19, 4, 4, R, 0, true),
        // row 2
        grid(20, 2, 4, L, 0, false),
        grid(21, 2, 3, L, 1, false),
        grid(22, 2, 2, L, 2, false),
        grid(23, 2, 1, L, 3, false),
        grid(24, 2, 0, L, 4, false),
        grid(25, 5, 0, R, 4, false),
        grid(26, 5, 1, R, 3, false),
        grid(27, 5, 2, R, 2, false),
        grid(28, 5, 3, R, 1, false),
        grid(29, 5, 4, R, 0, false),
        // thumbs: left outer → inner, right inner → outer
        thumb(30, 0, 5, L, 0),
        thumb(31, 1, 5, L, 1),
        thumb(32, 2, 5, L, 2),
        thumb(33, 5, 5, R, 2),
        thumb(34, 4, 5, R, 1),
        thumb(35, 3, 5, R, 0),
    ]
};

#[cfg(test)] // only tests use this
pub fn key_at(row: u8, col: u8) -> Option<&'static PhysicalKey> {
    OP36_KEYS.iter().find(|k| k.row == row && k.col == col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op36_has_36_unique_matrix_positions() {
        let mut seen = std::collections::HashSet::new();
        for k in OP36_KEYS.iter() {
            assert!(k.row < 6 && k.col < 6, "key {} out of matrix", k.index);
            assert!(
                seen.insert((k.row, k.col)),
                "duplicate matrix position {:?}",
                (k.row, k.col)
            );
        }
        assert_eq!(seen.len(), 36);
        for (i, k) in OP36_KEYS.iter().enumerate() {
            assert_eq!(k.index as usize, i);
        }
    }

    #[test]
    fn reading_order_matches_spec() {
        assert_eq!((OP36_KEYS[0].row, OP36_KEYS[0].col), (0, 4)); // Q
        assert_eq!((OP36_KEYS[9].row, OP36_KEYS[9].col), (3, 4)); // P
        assert_eq!((OP36_KEYS[30].row, OP36_KEYS[30].col), (0, 5)); // Bksp
        assert_eq!((OP36_KEYS[33].row, OP36_KEYS[33].col), (5, 5)); // Enter (right inner)
        assert_eq!((OP36_KEYS[35].row, OP36_KEYS[35].col), (3, 5)); // Tab
    }

    #[test]
    fn home_row_flags() {
        let home: Vec<u8> = OP36_KEYS
            .iter()
            .filter(|k| k.home_row)
            .map(|k| k.index)
            .collect();
        assert_eq!(home, vec![10, 11, 12, 13, 16, 17, 18, 19]);
    }

    #[test]
    fn hands_columns_and_thumbs() {
        assert_eq!(
            (OP36_KEYS[0].hand, OP36_KEYS[0].column, OP36_KEYS[0].thumb),
            (Hand::Left, Some(0), None)
        );
        assert_eq!(
            (OP36_KEYS[4].hand, OP36_KEYS[4].column),
            (Hand::Left, Some(4))
        );
        assert_eq!(
            (OP36_KEYS[5].hand, OP36_KEYS[5].column),
            (Hand::Right, Some(4))
        );
        assert_eq!(
            (OP36_KEYS[9].hand, OP36_KEYS[9].column),
            (Hand::Right, Some(0))
        );
        assert_eq!(
            (OP36_KEYS[30].hand, OP36_KEYS[30].thumb),
            (Hand::Left, Some(0))
        ); // outer
        assert_eq!(
            (OP36_KEYS[32].hand, OP36_KEYS[32].thumb),
            (Hand::Left, Some(2))
        ); // inner
        assert_eq!(
            (OP36_KEYS[33].hand, OP36_KEYS[33].thumb),
            (Hand::Right, Some(2))
        ); // inner
        assert_eq!(
            (OP36_KEYS[35].hand, OP36_KEYS[35].thumb),
            (Hand::Right, Some(0))
        ); // outer
    }

    #[test]
    fn key_at_finds_positions() {
        assert_eq!(key_at(0, 4).map(|k| k.index), Some(0));
        assert_eq!(key_at(4, 5).map(|k| k.index), Some(34));
        assert_eq!(key_at(0, 0).map(|k| k.index), Some(4));
        assert!(key_at(6, 0).is_none());
    }
}
