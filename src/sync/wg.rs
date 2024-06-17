use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// WaitGroup impl use channel,it's also both support sync and async
/// how to use?
///
/// * on tokio
/// ```rust
/// use dark_std::sync::WaitGroup;
/// use std::time::Duration;
/// use tokio::time::sleep;
/// #[tokio::main]
/// async fn main() {
///     let wg = WaitGroup::new();
///     let wg2 = wg.clone();
///     tokio::spawn(async move {
///         sleep(Duration::from_secs(1)).await;
///         drop(wg2);
///     });
///     wg.wait_async().await;
///     println!("all done");
/// }
/// ```
/// * on thread
/// ```rust
/// use dark_std::sync::WaitGroup;
/// use std::time::Duration;
/// use std::thread::sleep;
///
/// fn main() {
///     let wg = WaitGroup::new();
///     let wg2 = wg.clone();
///     std::thread::spawn(move ||{
///         sleep(Duration::from_secs(1));
///         drop(wg2);
///     });
///     wg.wait();
///     println!("all done");
/// }
/// ```
pub struct WaitGroup {
    pub total: Arc<AtomicU64>,
    pub recv: Arc<flume::Receiver<u64>>,
    pub send: Arc<flume::Sender<u64>>,
}

impl Clone for WaitGroup {
    fn clone(&self) -> Self {
        self.add(1);
        Self {
            total: self.total.clone(),
            recv: self.recv.clone(),
            send: self.send.clone(),
        }
    }
}

impl WaitGroup {
    pub fn new() -> Self {
        let (s, r) = flume::unbounded();
        Self {
            total: Arc::new(AtomicU64::new(0)),
            recv: Arc::new(r),
            send: Arc::new(s),
        }
    }

    pub fn add(&self, v: u64) {
        let current = self.total.fetch_or(0, Ordering::SeqCst);
        self.total.store(current + v, Ordering::SeqCst);
    }

    pub async fn wait_async(&self) {
        if self.total.load(Ordering::Relaxed)==0{
            return;
        }
        let mut total;
        let mut current = 0;
        loop {
            match self.recv.recv_async().await {
                Ok(v) => {
                    current += v;
                    total = self.total.fetch_or(0, Ordering::SeqCst);
                    if current >= total {
                        break;
                    }
                }
                Err(_) => {
                    break;
                }
            }
        }
    }

    pub fn wait(&self) {
        if self.total.load(Ordering::Relaxed)==0{
            return;
        }
        let mut total;
        let mut current = 0;
        loop {
            match self.recv.recv() {
                Ok(v) => {
                    current += v;
                    total = self.total.fetch_or(0, Ordering::SeqCst);
                    if current >= total {
                        break;
                    }
                }
                Err(_) => {
                    break;
                }
            }
        }
    }
}

impl Drop for WaitGroup {
    fn drop(&mut self) {
        let _ = self.send.send(1);
    }
}
