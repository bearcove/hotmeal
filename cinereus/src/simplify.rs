//! Edit script simplification.
//!
//! Consolidates redundant operations to produce cleaner diffs:
//! - When a subtree is inserted, don't report individual child inserts
//! - When a subtree is deleted, don't report individual child deletes
//! - When a subtree is moved, don't report individual child moves

use crate::chawathe::EditOp;
use crate::tree::{Tree, TreeTypes};
use crate::{debug, trace};
use indextree::NodeId;
use std::collections::{HashMap, HashSet};

/// Simplify an edit script by consolidating subtree operations.
///
/// This removes redundant child operations when a parent operation already
/// covers the entire subtree.
pub fn simplify_edit_script<T: TreeTypes>(
    ops: Vec<EditOp<T>>,
    tree_a: &Tree<T>,
    tree_b: &Tree<T>,
) -> Vec<EditOp<T>> {
    debug!(ops_count = ops.len(), "simplify_edit_script start");

    // Collect all nodes involved in each operation type
    let mut inserted_nodes: HashSet<NodeId> = HashSet::new();
    let mut deleted_nodes: HashSet<NodeId> = HashSet::new();
    let mut moved_nodes_b: HashSet<NodeId> = HashSet::new();
    let mut move_pairs: HashMap<NodeId, NodeId> = HashMap::new(); // node_b -> node_a

    for op in &ops {
        match op {
            EditOp::Insert { node_b, .. } => {
                inserted_nodes.insert(*node_b);
            }
            EditOp::Delete { node_a } => {
                deleted_nodes.insert(*node_a);
            }
            EditOp::Move { node_a, node_b, .. } => {
                moved_nodes_b.insert(*node_b);
                move_pairs.insert(*node_b, *node_a);
            }
            EditOp::UpdateProperties { .. } => {}
        }
    }

    debug!(
        inserted = inserted_nodes.len(),
        deleted = deleted_nodes.len(),
        moved = moved_nodes_b.len(),
        "collected nodes"
    );

    // Find "root" operations - those whose parent is not also in the set
    let root_inserts: HashSet<NodeId> = inserted_nodes
        .iter()
        .filter(|&&node| {
            tree_b
                .parent(node)
                .map(|p| !inserted_nodes.contains(&p))
                .unwrap_or(true)
        })
        .copied()
        .collect();

    let root_deletes: HashSet<NodeId> = deleted_nodes
        .iter()
        .filter(|&&node| {
            tree_a
                .parent(node)
                .map(|p| !deleted_nodes.contains(&p))
                .unwrap_or(true)
        })
        .copied()
        .collect();

    // For moves, a child move is only dominated if the parent relationship exists
    // in BOTH tree_a and tree_b. Otherwise, the nodes are structurally unrelated
    // in the source tree and the child move must be preserved.
    let root_moves: HashSet<NodeId> = moved_nodes_b
        .iter()
        .filter(|&&node_b| {
            // Check if parent is being moved in tree_b
            let parent_b_moving = tree_b
                .parent(node_b)
                .map(|p_b| moved_nodes_b.contains(&p_b))
                .unwrap_or(false);

            if !parent_b_moving {
                // Parent not being moved in tree_b - this is a root move
                return true;
            }

            // Parent is being moved in tree_b. Check if they were ALSO parent-child in tree_a.
            let node_a = move_pairs.get(&node_b).copied().unwrap();
            let parent_a_in_tree_a = tree_a.parent(node_a);

            if let Some(parent_a) = parent_a_in_tree_a {
                // Find what parent_b maps to in tree_a
                let parent_b = tree_b.parent(node_b).unwrap();
                if let Some(&parent_a_from_move) = move_pairs.get(&parent_b) {
                    // Both nodes are being moved. Check if they were parent-child in tree_a.
                    if parent_a == parent_a_from_move {
                        // They were parent-child in BOTH trees - child move is dominated
                        return false;
                    }
                }
            }

            // Not dominated - this is a root move
            true
        })
        .copied()
        .collect();

    debug!(
        root_inserts = root_inserts.len(),
        root_deletes = root_deletes.len(),
        root_moves = root_moves.len(),
        "found root operations"
    );

    // Filter operations to only include roots
    let result: Vec<_> = ops
        .into_iter()
        .filter(|op| {
            let dominated = match op {
                EditOp::Insert { node_b, .. } => !root_inserts.contains(node_b),
                EditOp::Delete { node_a } => !root_deletes.contains(node_a),
                EditOp::Move { node_b, .. } => !root_moves.contains(node_b),
                EditOp::UpdateProperties { .. } => false, // Always keep property updates
            };
            if dominated {
                trace!("simplify: dropping dominated op");
            }
            !dominated
        })
        .collect();

    debug!(
        before = inserted_nodes.len() + deleted_nodes.len() + moved_nodes_b.len(),
        after = result.len(),
        "simplify_edit_script done"
    );

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{NodeData, SimpleTypes};

    type TestTypes = SimpleTypes<&'static str, String>;

    #[test]
    fn test_simplify_subtree_insert() {
        // Tree B has a subtree: parent -> child1, child2
        let mut tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(0, "root"));
        let parent = tree_b.add_child(tree_b.root, NodeData::simple_u64(1, "parent"));
        let child1 = tree_b.add_child(
            parent,
            NodeData::simple_leaf_u64(2, "leaf", "a".to_string()),
        );
        let child2 = tree_b.add_child(
            parent,
            NodeData::simple_leaf_u64(3, "leaf", "b".to_string()),
        );

        // Empty tree A for reference
        let tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(0, "root"));

        // Simulate raw ops: insert parent, insert child1, insert child2
        let ops: Vec<EditOp<TestTypes>> = vec![
            EditOp::Insert {
                node_b: parent,
                parent_b: tree_b.root,
                position: 0,
                kind: "parent",
                label: None,
            },
            EditOp::Insert {
                node_b: child1,
                parent_b: parent,
                position: 0,
                kind: "leaf",
                label: Some("a".to_string()),
            },
            EditOp::Insert {
                node_b: child2,
                parent_b: parent,
                position: 1,
                kind: "leaf",
                label: Some("b".to_string()),
            },
        ];

        let simplified = simplify_edit_script(ops, &tree_a, &tree_b);

        // Should only have the parent insert
        assert_eq!(simplified.len(), 1);
        assert!(matches!(
            &simplified[0],
            EditOp::Insert { node_b, .. } if *node_b == parent
        ));
    }

    #[test]
    fn test_simplify_subtree_delete() {
        // Tree A has a subtree: parent -> child1, child2
        let mut tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(0, "root"));
        let parent = tree_a.add_child(tree_a.root, NodeData::simple_u64(1, "parent"));
        let child1 = tree_a.add_child(
            parent,
            NodeData::simple_leaf_u64(2, "leaf", "a".to_string()),
        );
        let child2 = tree_a.add_child(
            parent,
            NodeData::simple_leaf_u64(3, "leaf", "b".to_string()),
        );

        // Empty tree B for reference
        let tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(0, "root"));

        // Simulate raw ops: delete child1, delete child2, delete parent (post-order)
        let ops: Vec<EditOp<TestTypes>> = vec![
            EditOp::Delete { node_a: child1 },
            EditOp::Delete { node_a: child2 },
            EditOp::Delete { node_a: parent },
        ];

        let simplified = simplify_edit_script(ops, &tree_a, &tree_b);

        // Should only have the parent delete
        assert_eq!(simplified.len(), 1);
        assert!(matches!(
            &simplified[0],
            EditOp::Delete { node_a } if *node_a == parent
        ));
    }

    #[test]
    fn test_simplify_keeps_independent_ops() {
        let mut tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(0, "root"));
        let a1 = tree_a.add_child(
            tree_a.root,
            NodeData::simple_leaf_u64(1, "leaf", "a".to_string()),
        );
        let a2 = tree_a.add_child(
            tree_a.root,
            NodeData::simple_leaf_u64(2, "leaf", "b".to_string()),
        );

        let tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(0, "root"));

        // Two independent deletes (siblings, not parent-child)
        let ops: Vec<EditOp<TestTypes>> =
            vec![EditOp::Delete { node_a: a1 }, EditOp::Delete { node_a: a2 }];

        let simplified = simplify_edit_script(ops, &tree_a, &tree_b);

        // Both should remain since they're independent
        assert_eq!(simplified.len(), 2);
    }
}
