//! Source-first tests for explicit bounded DOM graph links; compile/runtime UNRUN.
use super::*;

#[test]
fn named_chain_requires_explicit_source_and_previous_result() {
    assert!(
        validate_chain_links(&[(Some("SourceGraphic"), Some("a")), (Some("a"), Some("b"))]).is_ok()
    );
    assert!(validate_chain_links(&[(None, None)]).is_ok());
    assert!(validate_chain_links(&[(Some("SourceGraphic"), Some("a")), (Some("a"), None)]).is_ok());
    for links in [
        [(None, Some("a")), (Some("a"), Some("b"))],
        [(Some("SourceGraphic"), None), (Some("a"), Some("b"))],
        [(Some("SourceGraphic"), Some("a")), (None, Some("b"))],
        [
            (Some("SourceGraphic"), Some("a")),
            (Some("SourceGraphic"), Some("b")),
        ],
        [(Some("b"), Some("a")), (Some("a"), Some("b"))],
        [(Some("SourceGraphic"), Some("a")), (Some("b"), Some("b"))],
        [(Some("SourceGraphic"), Some("a")), (Some("a"), Some("a"))],
        [
            (Some("SourceGraphic"), Some("SourceAlpha")),
            (Some("SourceAlpha"), Some("b")),
        ],
    ] {
        assert!(matches!(
            validate_chain_links(&links),
            Err(FilterError::Unsupported(_))
        ));
    }
    assert!(validate_chain_links(&[(None, None); 3]).is_err());
    assert!(validate_chain_links(&[]).is_err());
}

#[test]
fn explicit_stage_region_and_final_crop_have_distinct_coordinates() {
    let final_crop = Rect::new(80.0, 8.0, 168.0, 88.0);
    assert_eq!(
        stage_region(
            [Some("120"), Some("44"), Some("4"), Some("8")],
            final_crop,
            Rect::new(120.0, 44.0, 128.0, 52.0),
            false
        )
        .unwrap(),
        Rect::new(120.0, 44.0, 124.0, 52.0)
    );
    assert_eq!(
        stage_region([None; 4], final_crop, Rect::ZERO, false).unwrap(),
        final_crop
    );
    assert!(
        stage_region(
            [None, None, Some("-1"), None],
            final_crop,
            Rect::ZERO,
            false
        )
        .is_err()
    );
    assert!(
        stage_region(
            [Some("NaN"), None, None, None],
            final_crop,
            Rect::ZERO,
            false
        )
        .is_err()
    );
}
