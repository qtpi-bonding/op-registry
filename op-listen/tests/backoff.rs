use std::time::Duration;

use op_listen::Backoff;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn take(b: &mut Backoff, n: usize) -> Vec<Duration> {
    (0..n).map(|_| b.next_delay()).collect()
}

#[test]
fn follower_sequence_is_250_500_to_the_cap() {
    let mut b = Backoff::follower();
    let want: Vec<Duration> = [250, 500, 1000, 2000, 4000, 8000, 16000, 30000, 30000]
        .into_iter()
        .map(ms)
        .collect();
    assert_eq!(take(&mut b, 9), want);
}

#[test]
fn link_sequence_is_1_2_4_8_16_30_30() {
    let mut b = Backoff::link();
    let want: Vec<Duration> = [1, 2, 4, 8, 16, 30, 30].into_iter().map(secs).collect();
    assert_eq!(take(&mut b, 7), want);
}

#[test]
fn reset_restarts_the_sequence() {
    let mut b = Backoff::link();
    assert_eq!(take(&mut b, 3), vec![secs(1), secs(2), secs(4)]);
    b.reset();
    assert_eq!(take(&mut b, 2), vec![secs(1), secs(2)]);
}

#[test]
fn first_above_cap_is_clamped() {
    let mut b = Backoff::new(secs(60), secs(30));
    assert_eq!(take(&mut b, 3), vec![secs(30), secs(30), secs(30)]);
}

#[test]
fn connection_ended_resets_only_after_the_healthy_period() {
    let mut b = Backoff::new(secs(1), secs(30)).with_healthy_after(secs(10));
    b.next_delay();
    b.next_delay();
    b.connection_ended(secs(9));
    assert_eq!(b.next_delay(), secs(4));
    b.connection_ended(secs(10));
    assert_eq!(b.next_delay(), secs(1));
}

#[test]
fn grorg_client_shape() {
    let mut b = Backoff::new(ms(200), secs(5));
    let want: Vec<Duration> = [200, 400, 800, 1600, 3200, 5000, 5000]
        .into_iter()
        .map(ms)
        .collect();
    assert_eq!(take(&mut b, 7), want);
    b.reset();
    assert_eq!(b.next_delay(), ms(200));
}
