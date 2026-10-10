use herdr_relay::timing;
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn a_timeline_reports_each_phase_and_the_total() {
    let (value, timeline) = timing::record("attach", async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        timing::mark("challenge");
        tokio::time::sleep(Duration::from_millis(15)).await;
        timing::mark("route");
        7
    })
    .await;
    assert_eq!(value, 7);
    assert_eq!(
        timeline.render(),
        "attach total=55ms challenge=40ms route=15ms"
    );
}

#[tokio::test]
async fn marks_outside_a_timeline_are_ignored() {
    timing::mark("orphan");
}

#[tokio::test(start_paused = true)]
async fn nested_operations_keep_their_own_phases() {
    let (_, outer) = timing::record("connect", async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let (_, inner) = timing::record("targets", async {
            tokio::time::sleep(Duration::from_millis(3)).await;
            timing::mark("control");
        })
        .await;
        assert_eq!(inner.render(), "targets total=3ms control=3ms");
        timing::mark("listed");
    })
    .await;
    assert_eq!(outer.render(), "connect total=8ms listed=8ms");
}
