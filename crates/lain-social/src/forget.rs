//! Forgetting a deleted account: every social row it owns, and its
//! traces in other accounts' rows (edges, notifications it sent, shares
//! of collections). The account's id derives from its name, so a row
//! left behind would belong to the next account of that name.

use lain_store::Bucket;

use crate::{Collection, Comment, Error, Notification, Social, Tx, pair, prefix, scan, work_key};

impl Social {
    /// Removes `user_id`'s settings, relationships (both directions),
    /// activity, notifications (received and sent), ratings, comments
    /// and collections, and takes the account off collections shared
    /// with it. Answers how many rows went.
    pub fn forget_user(&self, user_id: &str) -> Result<i64, Error> {
        self.update(|tx| {
            let mut removed = 0i64;
            let mut del = |tx: &mut Tx<'_>, b: Bucket, k: &[u8]| -> Result<(), Error> {
                if tx.delete(b, k)? {
                    removed += 1;
                }
                Ok(())
            };
            let own = prefix(user_id);
            del(tx, Bucket::SocialSettings, user_id.as_bytes())?;
            for b in [Bucket::SocialActivity, Bucket::SocialActivityLast, Bucket::SocialNotifications] {
                for (k, _) in tx.w.scan_prefix(b, &own).map_err(crate::store_err)? {
                    del(tx, b, &k)?;
                }
            }
            // Edges are `owner\0other`: this account's own, and every
            // other account's edge pointing at it.
            for (k, _) in tx.w.all(Bucket::SocialEdges).map_err(crate::store_err)? {
                if k.starts_with(&own) || k.ends_with(&[b"\0".as_slice(), user_id.as_bytes()].concat()) {
                    del(tx, Bucket::SocialEdges, &k)?;
                }
            }
            for (k, n) in scan::<Notification>(tx, Bucket::SocialNotifications, b"")? {
                if n.from_user_id == user_id {
                    del(tx, Bucket::SocialNotifications, k.as_bytes())?;
                }
            }
            for (work, _) in scan::<serde_json::Value>(tx, Bucket::SocialRatingsByUser, &own)? {
                del(tx, Bucket::SocialRatings, &pair(&work, user_id))?;
                del(tx, Bucket::SocialRatingsByUser, &pair(user_id, &work))?;
            }
            for (id, c) in scan::<Comment>(tx, Bucket::SocialComments, b"")? {
                if c.user_id == user_id {
                    del(tx, Bucket::SocialComments, id.as_bytes())?;
                    del(tx, Bucket::SocialCommentsByWork, &pair(&work_key(&c.work), &id))?;
                }
            }
            for (id, mut c) in scan::<Collection>(tx, Bucket::SocialCollections, b"")? {
                if c.owner_id == user_id {
                    del(tx, Bucket::SocialCollections, id.as_bytes())?;
                } else if c.shared_with.iter().any(|u| u == user_id) {
                    c.shared_with.retain(|u| u != user_id);
                    tx.put(Bucket::SocialCollections, id.as_bytes(), &c)?;
                }
            }
            Ok(removed)
        })
    }
}
