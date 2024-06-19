use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{error::BuilderError, CosmosBuilder};

use super::node_chooser::{AllNodes, NodeChooser};

#[derive(Clone)]
pub(super) struct Pool {
    pub(super) builder: Arc<CosmosBuilder>,
    pub(super) node_chooser: NodeChooser,
    /// Permits for enforcing global concurrent request count.
    semaphore: Arc<Semaphore>,
}

impl Pool {
    pub(super) fn new(builder: Arc<CosmosBuilder>) -> Result<Self, BuilderError> {
        let node_chooser = NodeChooser::new(&builder)?;
        let semaphore = Arc::new(Semaphore::new(builder.request_count()));
        Ok(Pool {
            builder,
            node_chooser,
            semaphore,
        })
    }

    pub(super) fn all_nodes(&self) -> AllNodes {
        self.node_chooser.all_nodes()
    }

    pub(crate) async fn get_node_permit(&self) -> OwnedSemaphorePermit {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("Pool::get_with_node: semaphore has been closed")
    }
}
