use axum::{
    extract::{Path, State},
    response::Html,
};

use crate::error::AppError;
use crate::nntp::GroupTreeNode;
use crate::state::AppState;

pub async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await?;

    // Build tree hierarchy from flat group list
    let tree = GroupTreeNode::build_tree(&groups);

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("groups", &groups);
    context.insert("nodes", &tree);
    context.insert("path", "");
    context.insert("breadcrumbs", &Vec::<(&str, &str)>::new());

    let html = state.tera.render("home.html", &context)?;
    Ok(Html(html))
}

pub async fn browse(
    State(state): State<AppState>,
    Path(prefix): Path<String>,
) -> Result<Html<String>, AppError> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await?;

    // Build tree hierarchy
    let tree = GroupTreeNode::build_tree(&groups);

    // Find children at the given path
    let nodes = GroupTreeNode::find_children_at_path(&tree, &prefix)
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
    context.insert("nodes", &nodes);
    context.insert("path", &prefix);
    context.insert("breadcrumbs", &breadcrumbs);
    context.insert("current_node", &current_node);

    let html = state.tera.render("home.html", &context)?;
    Ok(Html(html))
}
