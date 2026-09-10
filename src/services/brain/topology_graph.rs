use serde::{Deserialize, Serialize};
use crate::services::brain::agent_blackboard::AgentBlackboard;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopologyNode {
    Planner,
    Coder,
    Reviewer,
    TerminalSuccess,
    TerminalFailure(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewDecision {
    pub approved: bool,
    pub feedback: String,
    pub target_redesign: bool, // true -> Planner, false -> Coder
}

#[derive(Debug, Clone)]
pub struct TopologyStateTransition {
    pub from: TopologyNode,
    pub to: TopologyNode,
    pub reason: String,
    pub timestamp: i64,
}

pub struct AgentTopologyGraph {
    current_node: TopologyNode,
    blackboard: AgentBlackboard,
    transition_history: Vec<TopologyStateTransition>,
    max_rejections: usize,
    rejection_count: usize,
}

impl AgentTopologyGraph {
    pub fn new(blackboard: AgentBlackboard) -> Self {
        Self {
            current_node: TopologyNode::Planner,
            blackboard,
            transition_history: Vec::new(),
            max_rejections: 3,
            rejection_count: 0,
        }
    }

    pub fn current_node(&self) -> &TopologyNode {
        &self.current_node
    }

    pub fn transition_to(&mut self, next: TopologyNode, reason: impl Into<String>) {
        let transition = TopologyStateTransition {
            from: self.current_node.clone(),
            to: next.clone(),
            reason: reason.into(),
            timestamp: chrono::Utc::now().timestamp(),
        };
        self.transition_history.push(transition);
        self.current_node = next;
    }

    /// Reviewer 节点的决策处理逻辑
    pub fn process_reviewer_decision(&mut self, decision: ReviewDecision) -> TopologyNode {
        if decision.approved {
            self.blackboard.post("review_result", format!("APPROVED: {}", decision.feedback), "Reviewer", "Approval");
            self.transition_to(TopologyNode::TerminalSuccess, "Reviewer approved solution");
        } else {
            self.rejection_count += 1;
            self.blackboard.post(
                format!("review_feedback_{}", self.rejection_count),
                &decision.feedback,
                "Reviewer",
                "Feedback",
            );

            if self.rejection_count > self.max_rejections {
                let fail_msg = format!("Exceeded max rejections ({})", self.max_rejections);
                self.transition_to(TopologyNode::TerminalFailure(fail_msg.clone()), "Max rejections exceeded");
            } else if decision.target_redesign {
                self.transition_to(TopologyNode::Planner, format!("Reviewer requested structural redesign: {}", decision.feedback));
            } else {
                self.transition_to(TopologyNode::Coder, format!("Reviewer requested code fixes: {}", decision.feedback));
            }
        }

        self.current_node.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topology_graph_flow() {
        let board = AgentBlackboard::new();
        let mut graph = AgentTopologyGraph::new(board.clone());

        assert_eq!(*graph.current_node(), TopologyNode::Planner);

        graph.transition_to(TopologyNode::Coder, "Plan approved");
        assert_eq!(*graph.current_node(), TopologyNode::Coder);

        graph.transition_to(TopologyNode::Reviewer, "Code generated");
        assert_eq!(*graph.current_node(), TopologyNode::Reviewer);

        // Reject -> back to Coder
        let next = graph.process_reviewer_decision(ReviewDecision {
            approved: false,
            feedback: "Syntax error on line 4".to_string(),
            target_redesign: false,
        });
        assert_eq!(next, TopologyNode::Coder);

        // Code again -> Reviewer -> Approve
        graph.transition_to(TopologyNode::Reviewer, "Fixed syntax error");
        let final_node = graph.process_reviewer_decision(ReviewDecision {
            approved: true,
            feedback: "Looks good".to_string(),
            target_redesign: false,
        });
        assert_eq!(final_node, TopologyNode::TerminalSuccess);

        assert!(board.get("review_result").is_some());
    }
}
