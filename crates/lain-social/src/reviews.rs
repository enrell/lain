//! `lain.social.reviews@1`: ratings, reviews and comments on works.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use lain_api::{activity_type, notify};
use lain_core::Error;
use lain_store::Bucket;

use crate::graph::{blocked_either, can_see_tx};
use crate::{
    Activity, Comment, Hidden, Rating, SUBJECT_RATINGS, Social, WorkRef, clean_text, forbidden, get_json, hidden_work,
    input, invalid, need_user, normalize_work, not_found, out, pair, prefix, scan, work_key,
};

/// Ops of `lain.social.reviews@1`.
pub mod reviews_op {
    /// `{viewer_id, work, hidden_libraries?}` → `WorkView`
    pub const WORK: &str = "work";
    /// `{user_id, work, score, review, spoiler}` → `Rating`
    pub const PUT_RATING: &str = "put-rating";
    /// `{user_id, work}` → bool (whether it existed)
    pub const DELETE_RATING: &str = "delete-rating";
    /// `{viewer_id, owner_id, hidden_libraries?}` → `[Rating]`
    pub const USER_RATINGS: &str = "user-ratings";
    /// `{user_id, work, body, reply_to?}` → `Comment`
    pub const ADD_COMMENT: &str = "add-comment";
    /// `{user_id, id, moderator?}` → true
    pub const DELETE_COMMENT: &str = "delete-comment";
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkInput {
    #[serde(default)]
    pub viewer_id: String,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

/// Everything the viewer may see about a work.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkView {
    pub work: WorkRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<Rating>,
    pub ratings: Vec<Rating>,
    pub average: f64,
    pub scored: i64,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PutRatingInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub review: String,
    #[serde(default)]
    pub spoiler: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeleteRatingInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub work: WorkRef,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserRatingsInput {
    #[serde(default)]
    pub viewer_id: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AddCommentInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub reply_to: String,
}

/// Authors delete their own comments, moderators (admins) any.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeleteCommentInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub moderator: bool,
}

impl Social {
    pub(crate) fn invoke_reviews(&self, op: &str, v: Value) -> Result<Value, Error> {
        use reviews_op::*;
        match op {
            WORK => {
                let i: WorkInput = input(v, "WorkInput")?;
                need_user(&[&i.viewer_id])?;
                out(self.work(&i.viewer_id, i.work, &i.hidden_libraries.into_iter().collect())?)
            }
            PUT_RATING => {
                let i: PutRatingInput = input(v, "PutRatingInput")?;
                need_user(&[&i.user_id])?;
                out(self.put_rating(i)?)
            }
            DELETE_RATING => {
                let i: DeleteRatingInput = input(v, "DeleteRatingInput")?;
                need_user(&[&i.user_id])?;
                out(self.delete_rating(&i.user_id, i.work)?)
            }
            USER_RATINGS => {
                let i: UserRatingsInput = input(v, "UserRatingsInput")?;
                need_user(&[&i.viewer_id, &i.owner_id])?;
                out(self.user_ratings(&i.viewer_id, &i.owner_id, &i.hidden_libraries.into_iter().collect())?)
            }
            ADD_COMMENT => {
                let i: AddCommentInput = input(v, "AddCommentInput")?;
                need_user(&[&i.user_id])?;
                out(self.add_comment(i)?)
            }
            DELETE_COMMENT => {
                let i: DeleteCommentInput = input(v, "DeleteCommentInput")?;
                need_user(&[&i.user_id])?;
                out(self.delete_comment(&i)?)
            }
            _ => Err(invalid(format!("unknown op {op} of lain.social.reviews@1"))),
        }
    }

    /// The viewer's view of a work. Ratings and comments naming an item
    /// of a hidden library are left out of the lists and the average.
    pub fn work(&self, viewer: &str, w: WorkRef, hidden: &Hidden) -> Result<WorkView, Error> {
        let w = normalize_work(w)?;
        let key = work_key(&w);
        let mut view = WorkView { work: w, ..WorkView::default() };
        self.view(|r| {
            let mut sum = 0i64;
            for (_, rt) in scan::<Rating>(r, Bucket::SocialRatings, &prefix(&key))? {
                if hidden_work(hidden, &rt.work) {
                    continue;
                }
                if rt.user_id == viewer {
                    view.mine = Some(rt.clone());
                } else {
                    if !can_see_tx(r, viewer, &rt.user_id, SUBJECT_RATINGS)? {
                        continue;
                    }
                    view.ratings.push(rt.clone());
                }
                if rt.score > 0 {
                    sum += rt.score;
                    view.scored += 1;
                }
            }
            if view.scored > 0 {
                view.average = sum as f64 / view.scored as f64;
            }
            view.ratings.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
            for (id, _) in scan::<Value>(r, Bucket::SocialCommentsByWork, &prefix(&key))? {
                let Some(c) = get_json::<Comment>(r, Bucket::SocialComments, id.as_bytes())? else { continue };
                if hidden_work(hidden, &c.work) {
                    continue;
                }
                if c.user_id != viewer && blocked_either(r, viewer, &c.user_id)? {
                    continue;
                }
                view.comments.push(c);
            }
            Ok(())
        })?;
        Ok(view)
    }

    /// Stores the account's rating; a new rating, a changed score or a
    /// first review records a `rated` activity row.
    pub fn put_rating(&self, i: PutRatingInput) -> Result<Rating, Error> {
        let w = normalize_work(i.work)?;
        if !(0..=10).contains(&i.score) {
            return Err(invalid("score must be 1..10, or 0 for a review without a score"));
        }
        let review = clean_text(&i.review, crate::MAX_REVIEW, "review", true)?;
        if i.score == 0 && review.is_empty() {
            return Err(invalid("a rating needs a score or a review"));
        }
        let key = work_key(&w);
        let now = self.now();
        self.update(|tx| {
            let prev: Option<Rating> = get_json(tx, Bucket::SocialRatings, &pair(&key, &i.user_id))?;
            let r = Rating {
                user_id: i.user_id.clone(),
                work: w.clone(),
                score: i.score,
                review,
                spoiler: i.spoiler,
                created_at: prev.as_ref().map(|p| p.created_at).unwrap_or(now),
                updated_at: now,
            };
            tx.put(Bucket::SocialRatings, &pair(&key, &i.user_id), &r)?;
            tx.put_raw(Bucket::SocialRatingsByUser, &pair(&i.user_id, &key), b"{}")?;
            let changed = match &prev {
                None => true,
                Some(p) => p.score != r.score || (p.review.is_empty() && !r.review.is_empty()),
            };
            if changed {
                self.append_activity_tx(
                    tx,
                    Activity {
                        user_id: i.user_id.clone(),
                        kind: activity_type::RATED.into(),
                        work: w.clone(),
                        score: r.score,
                        at: now,
                        ..Activity::default()
                    },
                )?;
            }
            Ok(r)
        })
    }

    /// Removes the account's rating; absent is not an error.
    pub fn delete_rating(&self, user_id: &str, w: WorkRef) -> Result<bool, Error> {
        let w = normalize_work(w)?;
        let key = work_key(&w);
        self.update(|tx| {
            let found = tx.delete(Bucket::SocialRatings, &pair(&key, user_id))?;
            tx.delete(Bucket::SocialRatingsByUser, &pair(user_id, &key))?;
            Ok(found)
        })
    }

    /// An account's ratings, newest first, when the viewer may see them;
    /// otherwise empty.
    pub fn user_ratings(&self, viewer: &str, owner: &str, hidden: &Hidden) -> Result<Vec<Rating>, Error> {
        let mut out = self.view(|r| {
            if !can_see_tx(r, viewer, owner, SUBJECT_RATINGS)? {
                return Ok(Vec::new());
            }
            let mut out = Vec::new();
            for (key, _) in scan::<Value>(r, Bucket::SocialRatingsByUser, &prefix(owner))? {
                if let Some(rt) = get_json::<Rating>(r, Bucket::SocialRatings, &pair(&key, owner))?
                    && !hidden_work(hidden, &rt.work)
                {
                    out.push(rt);
                }
            }
            Ok(out)
        })?;
        out.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(out)
    }

    /// Posts a comment. A reply must name a comment on the same work and
    /// notifies that comment's author.
    pub fn add_comment(&self, i: AddCommentInput) -> Result<Comment, Error> {
        let w = normalize_work(i.work)?;
        let body = clean_text(&i.body, crate::MAX_COMMENT, "comment", true)?;
        if body.is_empty() {
            return Err(invalid("comment is empty"));
        }
        let key = work_key(&w);
        let now = self.now();
        self.update(|tx| {
            let mut c = Comment {
                id: self.new_id(),
                user_id: i.user_id.clone(),
                work: w.clone(),
                body,
                reply_to: String::new(),
                created_at: now,
            };
            let mut parent_author = String::new();
            if !i.reply_to.is_empty() {
                let parent: Comment = get_json(tx, Bucket::SocialComments, i.reply_to.as_bytes())?
                    .filter(|p: &Comment| work_key(&p.work) == key)
                    .ok_or_else(|| not_found("unknown comment"))?;
                if blocked_either(tx, &i.user_id, &parent.user_id)? {
                    return Err(not_found("unknown comment"));
                }
                c.reply_to = parent.id.clone();
                parent_author = parent.user_id;
            }
            tx.put(Bucket::SocialComments, c.id.as_bytes(), &c)?;
            tx.put_raw(Bucket::SocialCommentsByWork, &pair(&key, &c.id), b"{}")?;
            if !c.reply_to.is_empty() && parent_author != i.user_id {
                let (work, cid) = (w.clone(), c.id.clone());
                self.notify_tx(tx, &parent_author, notify::REPLY, &i.user_id, |n| {
                    n.work = Some(work);
                    n.comment_id = cid;
                })?;
            }
            Ok(c)
        })
    }

    /// Removes a comment; replies stay, pointing at a removed one.
    pub fn delete_comment(&self, i: &DeleteCommentInput) -> Result<bool, Error> {
        self.update(|tx| {
            let c: Comment =
                get_json(tx, Bucket::SocialComments, i.id.as_bytes())?.ok_or_else(|| not_found("unknown comment"))?;
            if c.user_id != i.user_id && !i.moderator {
                return Err(forbidden("only the author or an admin can delete a comment"));
            }
            tx.delete(Bucket::SocialComments, i.id.as_bytes())?;
            tx.delete(Bucket::SocialCommentsByWork, &pair(&work_key(&c.work), &c.id))?;
            Ok(true)
        })
    }
}
