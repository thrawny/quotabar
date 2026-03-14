const DEFAULT: &str = "\
@define-color bg #1e1e1e;
@define-color bg_card #262626;
@define-color bg_card_hover #2e2e2e;
@define-color bg_card_selected #1c2a3a;
@define-color bg_subtle #232323;
@define-color bg_surface #2a2a2a;
@define-color bg_elevated #3a3a3a;
@define-color border #4a4a4a;
@define-color text #d4d4d4;
@define-color text_dim #808080;
@define-color accent #4a9eff;
@define-color success #73c991;
@define-color warning #e5c07b;
";

const MOLOKAI: &str = "\
@define-color bg #1c1c1c;
@define-color bg_card #252521;
@define-color bg_card_hover #2e2e28;
@define-color bg_card_selected #2a2025;
@define-color bg_subtle #33332c;
@define-color bg_surface #3a3a34;
@define-color bg_elevated #49483e;
@define-color border #555555;
@define-color text #f8f8f2;
@define-color text_dim #75715e;
@define-color accent #f92672;
@define-color success #a6e22e;
@define-color warning #e6db74;
";

pub fn get(name: &str) -> &'static str {
    match name {
        "molokai" => MOLOKAI,
        _ => DEFAULT,
    }
}
