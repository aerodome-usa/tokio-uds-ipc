use std::time::Duration;

/// The delay between retries starts at
/// `min_backoff`, and doubles on each retry, until it reaches `max_backoff`. If
/// the inner tasks at any point runs for longer than `current_backoff`, the
/// backoff will reset back to `min_backoff`.
pub async fn retry_with_backoff<T, F>(
    backoff_bounds: std::ops::RangeInclusive<Duration>,
    inner: impl Fn() -> F,
) -> T
where
    F: Future<Output = Option<T>>,
{
    let mut backoff = *backoff_bounds.start();
    loop {
        let sleep = tokio::time::sleep(backoff);

        if let Some(r) = inner().await {
            break r;
        }

        if sleep.is_elapsed() {
            // if we did run for at least backoff, reset and immediately retry.
            backoff = *backoff_bounds.start();
        } else {
            // otherwise, wait, double the backoff, then retry.
            sleep.await;
            backoff = backoff.saturating_mul(2).min(*backoff_bounds.end());
        }
    }
}

macro_rules! sink_via_unpin {
    ($field:tt) => {
        fn poll_ready(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            self.$field.poll_ready_unpin(cx)
        }

        fn start_send(mut self: Pin<&mut Self>, item: Request) -> Result<(), Self::Error> {
            self.$field.start_send_unpin(item)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            self.$field.poll_flush_unpin(cx)
        }

        fn poll_close(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            self.$field.poll_close_unpin(cx)
        }
    };
}

pub(crate) use sink_via_unpin;
