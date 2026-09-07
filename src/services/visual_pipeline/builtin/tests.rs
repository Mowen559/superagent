use super::*;

    use super::*;

    #[test]
    fn language_codes_are_normalized_per_adapter() {
        assert_eq!(normalize_language("zh_CN", true), "ZH-CN");
        assert_eq!(normalize_language("EN-US", false), "en-us");
    }
}
