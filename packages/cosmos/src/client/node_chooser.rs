use std::sync::Arc;

use crate::{
    error::{Action, BuilderError, NodeHealthLevel, NodeHealthReport, QueryErrorDetails},
    CosmosBuilder,
};

use super::node::Node;

#[derive(Clone)]
pub(super) struct NodeChooser {
    primary: Arc<Node>,
    fallbacks: Arc<[Node]>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct NodeScore {
    error_count: usize,
    is_fallback: bool,
}

impl NodeChooser {
    pub(super) fn new(builder: &CosmosBuilder) -> Result<Self, BuilderError> {
        Ok(NodeChooser {
            primary: Arc::new(builder.make_node(builder.grpc_url_arc(), false)?),
            fallbacks: builder
                .grpc_fallback_urls()
                .iter()
                .map(|fallback| builder.make_node(fallback, true))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        })
    }

    /// Choose a list of nodes to try, including fallbacks.
    ///
    /// We return a Vec, ordered so that the client should try them in
    /// succession.
    ///
    /// The priority of the nodes is given by:
    ///
    /// * Blocked nodes are always skipped.
    ///
    /// * Nodes are sorted by error count.
    ///
    /// * For nodes with the same error count, primary is used first.
    pub(super) fn choose_nodes(&self) -> Vec<Node> {
        let mut nodes = std::iter::once(&*self.primary)
            .chain(&*self.fallbacks)
            .filter_map(|node| match node.node_health_level() {
                NodeHealthLevel::Unblocked { error_count } => Some((
                    NodeScore {
                        error_count,
                        is_fallback: node.is_fallback(),
                    },
                    node.clone(),
                )),
                NodeHealthLevel::Blocked => None,
            })
            .collect::<Vec<_>>();
        nodes.sort_by_key(|(score, _)| *score);
        nodes.into_iter().map(|(_, node)| node).collect()
    }

    pub(super) fn health_report(&self) -> NodeHealthReport {
        NodeHealthReport {
            nodes: std::iter::once(self.primary.health_report())
                .chain(self.fallbacks.iter().map(|node| node.health_report()))
                .collect(),
        }
    }

    pub(super) fn all_nodes(&self) -> AllNodes {
        AllNodes {
            primary: Some(&self.primary),
            fallbacks: self.fallbacks.iter(),
        }
    }
}

pub(crate) enum QueryResult {
    Success,
    NetworkError {
        err: QueryErrorDetails,
        action: Action,
    },
    OtherError,
}

pub(crate) struct AllNodes<'a> {
    primary: Option<&'a Node>,
    fallbacks: std::slice::Iter<'a, Node>,
}

impl<'a> Iterator for AllNodes<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        match self.primary.take() {
            Some(primary) => Some(primary),
            None => self.fallbacks.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_score_order() {
        assert!(
            NodeScore {
                error_count: 0,
                is_fallback: false
            } < NodeScore {
                error_count: 0,
                is_fallback: true
            }
        );
        assert!(
            NodeScore {
                error_count: 1,
                is_fallback: false
            } > NodeScore {
                error_count: 0,
                is_fallback: true
            }
        );
        assert!(
            NodeScore {
                error_count: 1,
                is_fallback: false
            } < NodeScore {
                error_count: 1,
                is_fallback: true
            }
        );
    }
}
