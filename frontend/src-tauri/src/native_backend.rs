use maxgamestudio_core::native_http::NativeHttpServer;
use std::io;
use tauri::async_runtime::{self, JoinHandle};
use tokio::sync::oneshot;

pub(crate) struct NativeBackend {
    stop: oneshot::Sender<()>,
    task: JoinHandle<io::Result<()>>,
}

impl NativeBackend {
    pub(crate) fn start(server: NativeHttpServer) -> Self {
        let (stop, stopped) = oneshot::channel();
        let task = async_runtime::spawn(server.serve(async {
            let _ = stopped.await;
        }));
        Self { stop, task }
    }

    pub(crate) fn running(&self) -> bool {
        !self.task.inner().is_finished()
    }

    pub(crate) fn stop(self) -> Result<(), String> {
        let _ = self.stop.send(());
        async_runtime::block_on(self.task)
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
}
