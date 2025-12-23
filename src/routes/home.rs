use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    response::Html,
};

use crate::error::AppError;
use crate::nntp::GroupTreeNode;
use crate::state::AppState;

/// Extract all group names from a list of tree nodes (recursively including children)
fn extract_all_group_names(nodes: &[GroupTreeNode]) -> Vec<String> {
    let mut names = Vec::new();
    for node in nodes {
        if let Some(ref name) = node.full_name {
            names.push(name.clone());
        }
        names.extend(extract_all_group_names(&node.children));
    }
    names
}

/// Get cached stats for groups and identify which need prefetching.
/// Returns: (cached group stats, thread counts, groups needing prefetch)
async fn get_stats_for_groups(
    state: &AppState,
    group_names: &[String],
) -> (HashMap<String, Option<String>>, HashMap<String, usize>, Vec<String>) {
    let mut group_stats: HashMap<String, Option<String>> = HashMap::new();
    let mut needs_prefetch: Vec<String> = Vec::new();

    for name in group_names {
        if let Some(stats) = state.nntp.get_cached_group_stats(name).await {
            group_stats.insert(name.clone(), stats.last_article_date);
        } else {
            needs_prefetch.push(name.clone());
        }
    }

    // Get cached thread counts (from threads cache - only populated after visiting a group)
    let thread_counts = state.nntp.get_all_cached_thread_counts_for(group_names).await;

    (group_stats, thread_counts, needs_prefetch)
}

pub async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await?;

    // Build tree hierarchy
    let tree = GroupTreeNode::build_tree(&groups);

    // Extract all group names from the tree
    let all_group_names = extract_all_group_names(&tree);

    // Get cached stats + identify what needs prefetching
    let (group_stats, thread_counts, needs_prefetch) =
        get_stats_for_groups(&state, &all_group_names).await;

    // Trigger background prefetch for uncached groups
    if !needs_prefetch.is_empty() {
        state.nntp.prefetch_group_stats(needs_prefetch);
    }

    // Build tree with available stats
    let tree_with_stats =
        GroupTreeNode::build_tree_with_stats(&groups, &thread_counts, &group_stats);

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("groups", &groups);
    context.insert("nodes", &tree_with_stats);
    context.insert("path", "");
    context.insert("breadcrumbs", &Vec::<(&str, &str)>::new());
    context.insert("group_stats", &group_stats);
    context.insert("thread_counts", &thread_counts);

    let html = state.tera.render("home.html", &context)?;
    Ok(Html(html))
}

pub async fn browse(
    State(state): State<AppState>,
    Path(prefix): Path<String>,
) -> Result<Html<String>, AppError> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await?;

    // Build initial tree to find which groups are visible at this path
    let initial_tree = GroupTreeNode::build_tree(&groups);
    let visible_nodes =
        GroupTreeNode::find_children_at_path(&initial_tree, &prefix).unwrap_or_default();

    // Also check if the current path itself is a group
    let current_node = GroupTreeNode::find_node_at_path(&initial_tree, &prefix);

    // Collect all group names from visible nodes + current node
    let mut all_group_names = extract_all_group_names(&visible_nodes);
    if let Some(ref node) = current_node {
        if let Some(ref name) = node.full_name {
            if !all_group_names.contains(name) {
                all_group_names.push(name.clone());
            }
        }
    }

    // Get cached stats + identify what needs prefetching
    let (group_stats, thread_counts, needs_prefetch) =
        get_stats_for_groups(&state, &all_group_names).await;

    // Trigger background prefetch for uncached groups
    if !needs_prefetch.is_empty() {
        state.nntp.prefetch_group_stats(needs_prefetch);
    }

    // Build tree hierarchy with stats
    let tree = GroupTreeNode::build_tree_with_stats(&groups, &thread_counts, &group_stats);

    // Find children at the given path
    let nodes_with_stats = GroupTreeNode::find_children_at_path(&tree, &prefix)
        .ok_or_else(|| AppError::Internal(format!("Path not found: {}", prefix)))?;

    // Find the current node (to check if it's also a group)
    let current_node = GroupTreeNode::find_node_at_path(&tree, &prefix);

    // Build breadcrumbs
    let parts: Vec<&str> = prefix.split('.').collect();
    let mut breadcrumbs: Vec<(String, String)> = Vec::new();
    let mut accumulated = String::new();
    for part in &parts {
        if !accumulated.is_empty() {
            accumulated.push('.');
        }
        accumulated.push_str(part);
        breadcrumbs.push((part.to_string(), accumulated.clone()));
    }

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("groups", &groups);
    context.insert("nodes", &nodes_with_stats);
    context.insert("path", &prefix);
    context.insert("breadcrumbs", &breadcrumbs);
    context.insert("current_node", &current_node);
    context.insert("group_stats", &group_stats);
    context.insert("thread_counts", &thread_counts);

    let html = state.tera.render("home.html", &context)?;
    Ok(Html(html))
}
