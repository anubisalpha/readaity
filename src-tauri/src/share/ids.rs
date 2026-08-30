//! Opaque per-book tokens. A client only ever sees `id`, never a path.
//! `id = url-safe-base64( HMAC-SHA256(session_key, path)[..16] )`.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::db::ShareBook;

type HmacSha = Hmac<Sha256>;

pub fn book_id(key: &[u8; 32], path: &str) -> String {
    let mut mac = HmacSha::new_from_slice(key).expect("hmac key");
    mac.update(path.as_bytes());
    let tag = mac.finalize().into_bytes();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&tag[..16])
}

/// Find the book an `id` refers to by recomputing every candidate's id.
pub fn find<'a>(key: &[u8; 32], books: &'a [ShareBook], id: &str) -> Option<&'a ShareBook> {
    books.iter().find(|b| book_id(key, &b.path) == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(path: &str) -> ShareBook {
        ShareBook {
            path: path.into(),
            title: "t".into(),
            format: "txt".into(),
            size: 1,
            page_count: 0,
            md5: None,
            has_cover: false,
        }
    }

    #[test]
    fn id_is_opaque_stable_and_key_dependent() {
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        let p = r"K:\eBooks\Novels\Some Book.epub";
        let id = book_id(&k1, p);
        assert_eq!(id, book_id(&k1, p)); // stable
        assert_ne!(id, book_id(&k2, p)); // key-dependent
        assert!(!id.contains('/') && !id.contains('\\') && !id.contains(':'));
        assert!(!id.is_empty());
    }

    #[test]
    fn find_resolves_only_the_right_book() {
        let k = [9u8; 32];
        let books = vec![book("a/one.txt"), book("b/two.txt")];
        let id = book_id(&k, "b/two.txt");
        assert_eq!(find(&k, &books, &id).unwrap().path, "b/two.txt");
        assert!(find(&k, &books, "AAAAAAAAAAAAAAAAAAAAAA").is_none());
    }
}
