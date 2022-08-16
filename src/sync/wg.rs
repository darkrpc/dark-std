use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};


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