#[path = "support/studio_preview.rs"]
mod fixture;

#[tokio::test]
async fn native_preview_fixture_obtains_real_actor_index_and_flipnote_results() {
    for art in [false, true] {
        let f = fixture::PreviewFixture::new(art).await;
        f.wait_ready().await;
        assert!(!f.group.is_empty());
        assert_ne!(f.device.to_string(), f.author);
        f.shutdown().await;
    }
}
