use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    error::{BuilderError, ConnectionError},
    CosmosBuilder,
};

use super::{
    node::Node,
    node_chooser::{AllNodes, NodeChooser},
};

#[derive(Clone)]
pub(super) struct Pool {
    pub(super) builder: Arc<CosmosBuilder>,
    pub(super) node_chooser: NodeChooser,
    /// Permits for enforcing global concurrent request count.
    semaphore: Arc<Semaphore>,
}

pub(super) struct NodeGuard {
    pub(super) inner: Node,
    _permit: OwnedSemaphorePermit,
}

impl NodeGuard {
    pub(crate) fn get_inner_mut(&mut self) -> &mut Node {
        &mut self.inner
    }
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

    pub(super) async fn get(&self) -> Result<NodeGuard, ConnectionError> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("Pool::get: semaphore has been closed");

        let node = self.node_chooser.choose_node()?;
        Ok(NodeGuard {
            inner: node.clone(),
            _permit: permit,
        })
    }

    pub(super) fn all_nodes(&self) -> AllNodeGuards {
        AllNodeGuards {
            pool: self,
            all_nodes: self.node_chooser.all_nodes(),
        }
    }

    pub(crate) async fn get_with_node(&self, node: &Node) -> Result<NodeGuard, ConnectionError> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("Pool::get_with_node: semaphore has been closed");

        Ok(NodeGuard {
            inner: node.clone(),
            _permit: permit,
        })
    }
}

pub(crate) struct AllNodeGuards<'a> {
    pool: &'a Pool,
    all_nodes: AllNodes<'a>,
}

impl AllNodeGuards<'_> {
    pub(crate) async fn next(&mut self) -> Option<NodeGuard> {
        let inner = self.all_nodes.next()?.clone();
        let _permit = self
            .pool
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("AllNodeGuards::next: semaphore has been closed");

        Some(NodeGuard { inner, _permit })
    }
}
