//! Placeholder roles grouped into the classes OOXML matches and styles them by, which every format's roles map
//! onto.

use slidock_ir::elements::PlaceholderRole;

/// What a placeholder binds to on its parent container: titles to a title, content to a body, the rest by role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoleClass {
    Title,
    Body,
    Dt,
    Ftr,
    SldNum,
    Hdr,
    Notes,
    Other,
}

pub fn role_class(role: PlaceholderRole) -> RoleClass {
    match role {
        PlaceholderRole::Title | PlaceholderRole::CtrTitle => RoleClass::Title,
        PlaceholderRole::Body
        | PlaceholderRole::SubTitle
        | PlaceholderRole::Obj
        | PlaceholderRole::Pic
        | PlaceholderRole::Chart
        | PlaceholderRole::Tbl
        | PlaceholderRole::Media => RoleClass::Body,
        PlaceholderRole::Dt => RoleClass::Dt,
        PlaceholderRole::Ftr => RoleClass::Ftr,
        PlaceholderRole::SldNum => RoleClass::SldNum,
        PlaceholderRole::Hdr => RoleClass::Hdr,
        PlaceholderRole::Notes => RoleClass::Notes,
        PlaceholderRole::Other => RoleClass::Other,
    }
}

/// The master text styles a placeholder's text cascades from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextStyleClass {
    Title,
    Body,
    Other,
}

/// The text style class of a placeholder role; text outside placeholders uses the presentation defaults only.
pub fn text_style_class(role: Option<PlaceholderRole>) -> Option<TextStyleClass> {
    Some(match role_class(role?) {
        RoleClass::Title => TextStyleClass::Title,
        RoleClass::Body => TextStyleClass::Body,
        _ => TextStyleClass::Other,
    })
}

pub fn is_title_role(role: Option<PlaceholderRole>) -> bool {
    role.is_some_and(|role| role_class(role) == RoleClass::Title)
}

/// The role as the IR writes it (`ctrTitle`, `sldNum`).
pub(crate) fn role_name(role: PlaceholderRole) -> &'static str {
    match role {
        PlaceholderRole::Title => "title",
        PlaceholderRole::CtrTitle => "ctrTitle",
        PlaceholderRole::SubTitle => "subTitle",
        PlaceholderRole::Body => "body",
        PlaceholderRole::Obj => "obj",
        PlaceholderRole::Pic => "pic",
        PlaceholderRole::Chart => "chart",
        PlaceholderRole::Tbl => "tbl",
        PlaceholderRole::Media => "media",
        PlaceholderRole::Dt => "dt",
        PlaceholderRole::Ftr => "ftr",
        PlaceholderRole::SldNum => "sldNum",
        PlaceholderRole::Hdr => "hdr",
        PlaceholderRole::Notes => "notes",
        PlaceholderRole::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLES: [PlaceholderRole; 15] = [
        PlaceholderRole::Title,
        PlaceholderRole::CtrTitle,
        PlaceholderRole::SubTitle,
        PlaceholderRole::Body,
        PlaceholderRole::Obj,
        PlaceholderRole::Pic,
        PlaceholderRole::Chart,
        PlaceholderRole::Tbl,
        PlaceholderRole::Media,
        PlaceholderRole::Dt,
        PlaceholderRole::Ftr,
        PlaceholderRole::SldNum,
        PlaceholderRole::Hdr,
        PlaceholderRole::Notes,
        PlaceholderRole::Other,
    ];

    #[test]
    fn classes_titles_content_and_the_rest() {
        let classes: Vec<RoleClass> = ROLES.iter().map(|&role| role_class(role)).collect();
        use RoleClass::*;
        assert_eq!(
            classes,
            [
                Title, Title, Body, Body, Body, Body, Body, Body, Body, Dt, Ftr, SldNum, Hdr,
                Notes, Other
            ]
        );
        assert_eq!(text_style_class(None), None);
        assert_eq!(
            text_style_class(Some(PlaceholderRole::SldNum)),
            Some(TextStyleClass::Other)
        );
        assert_eq!(
            text_style_class(Some(PlaceholderRole::Pic)),
            Some(TextStyleClass::Body)
        );
        assert!(is_title_role(Some(PlaceholderRole::CtrTitle)));
        assert!(!is_title_role(Some(PlaceholderRole::SubTitle)));
        assert!(!is_title_role(None));
    }

    #[test]
    fn names_roles_as_the_ir_writes_them() {
        for role in ROLES {
            let wire = serde_json::to_value(role).unwrap();
            assert_eq!(wire.as_str(), Some(role_name(role)));
        }
    }
}
