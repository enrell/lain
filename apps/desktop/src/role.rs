//! How the desktop reads `role`, and the small rules of the access
//! controls on Settings › Users and Libraries. Only the exact string
//! `admin` is an administrator. `manager`, `user` and anything this
//! build does not know are not: an unknown role must not become an
//! admin. The desktop has no manager surface of its own; it only lets an
//! admin give the role.

pub fn is_admin_role(role: &str) -> bool {
    role == "admin"
}

/// The role a "Make …" button on another account offers next:
/// user, manager, admin, and back to user. A role this build does not
/// know goes to `user`, never up.
pub fn next_role(role: &str) -> &'static str {
    match role {
        "user" => "manager",
        "manager" => "admin",
        _ => "user",
    }
}

/// The audience levels, least to most permissive.
pub const AUDIENCES: [&str; 3] = ["everyone", "teen", "mature"];

/// The level after `level` on a cycling button. An empty or unknown
/// level reads as the one the server would apply to it.
pub fn next_audience(level: &str) -> &'static str {
    let i = AUDIENCES.iter().position(|a| *a == level).unwrap_or(AUDIENCES.len() - 1);
    AUDIENCES[(i + 1) % AUDIENCES.len()]
}

/// What an account's allow-list becomes when `id` is switched. `None` is
/// "every library". Switching one off from "every library" starts from
/// all of them; a list that ends up holding every library is "every
/// library" again, so a library added later reaches the account. Ids the
/// caller cannot name (a locked private library) are kept as they are.
pub fn toggle_library(current: &Option<Vec<String>>, all: &[String], id: &str) -> Option<Vec<String>> {
    let mut list: Vec<String> = match current {
        Some(l) => l.clone(),
        None => all.to_vec(),
    };
    if let Some(i) = list.iter().position(|x| x == id) {
        list.remove(i);
    } else {
        list.push(id.to_string());
    }
    if all.iter().all(|a| list.contains(a)) { None } else { Some(list) }
}

#[cfg(test)]
mod tests {
    use super::{AUDIENCES, is_admin_role, next_audience, next_role, toggle_library};

    #[test]
    fn an_unknown_role_is_not_an_admin() {
        assert!(is_admin_role("admin"));
        assert!(!is_admin_role("manager"));
        assert!(!is_admin_role("user"));
        assert!(!is_admin_role(""));
        assert!(!is_admin_role("root"));
        assert!(!is_admin_role("Admin"));
    }

    #[test]
    fn roles_cycle_and_an_unknown_one_never_goes_up() {
        assert_eq!(next_role("user"), "manager");
        assert_eq!(next_role("manager"), "admin");
        assert_eq!(next_role("admin"), "user");
        for odd in ["", "root", "Admin"] {
            assert_eq!(next_role(odd), "user");
        }
    }

    #[test]
    fn audiences_cycle_and_an_unknown_one_reads_as_mature() {
        assert_eq!(AUDIENCES, ["everyone", "teen", "mature"]);
        assert_eq!(next_audience("everyone"), "teen");
        assert_eq!(next_audience("teen"), "mature");
        assert_eq!(next_audience("mature"), "everyone");
        assert_eq!(next_audience(""), "everyone");
        assert_eq!(next_audience("x-rated"), "everyone");
    }

    #[test]
    fn an_allow_list_switches_one_library_at_a_time() {
        let all: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let l = |v: &[&str]| Some(v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        // Off from "every library": every other one.
        assert_eq!(toggle_library(&None, &all, "b"), l(&["a", "c"]));
        // On again, and the full set is "every library" once more.
        assert_eq!(toggle_library(&l(&["a", "c"]), &all, "b"), None);
        // One by one from nothing.
        assert_eq!(toggle_library(&l(&[]), &all, "a"), l(&["a"]));
        assert_eq!(toggle_library(&l(&["a"]), &all, "a"), l(&[]));
        // An id the screen cannot name stays where it is.
        assert_eq!(toggle_library(&l(&["hidden", "a"]), &all, "b"), l(&["hidden", "a", "b"]));
        assert_eq!(toggle_library(&l(&["hidden", "a", "b"]), &all, "c"), None);
    }
}
