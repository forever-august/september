mod messages;
mod service;
mod worker;

pub use service::NntpService;

use nntp_rs::threading::{FetchedArticle, Thread, ThreadCollection, ThreadNode, ThreadedArticleRef};
use serde::Serialize;

/// View model for a thread in list view
#[derive(Debug, Clone, Serialize)]
pub struct ThreadView {
    pub subject: String,
    pub root_message_id: String,
    pub article_count: usize,
    pub root: ThreadNodeView,
    /// Date of the most recent post in the thread
    pub last_post_date: Option<String>,
}

/// View model for a node in the thread tree
#[derive(Debug, Clone, Serialize)]
pub struct ThreadNodeView {
    pub message_id: String,
    pub article: Option<ArticleView>,
    pub replies: Vec<ThreadNodeView>,
}

/// Flattened comment for non-recursive template rendering
#[derive(Debug, Clone, Serialize)]
pub struct FlatComment {
    pub message_id: String,
    pub article: Option<ArticleView>,
    pub depth: usize,
    /// Number of descendant replies (for collapse UI)
    pub descendant_count: usize,
    /// Whether this comment starts a collapsed section
    pub starts_collapsed: bool,
}

impl ThreadNodeView {
    /// Collect all message IDs in the tree (iteratively)
    pub fn collect_message_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        let mut stack: Vec<&ThreadNodeView> = vec![self];

        while let Some(node) = stack.pop() {
            ids.push(node.message_id.clone());
            for reply in &node.replies {
                stack.push(reply);
            }
        }

        ids
    }

    /// Populate article bodies from a map of message_id -> ArticleView (iteratively)
    pub fn populate_bodies(&mut self, articles: &std::collections::HashMap<String, ArticleView>) {
        let mut stack: Vec<&mut ThreadNodeView> = vec![self];

        while let Some(node) = stack.pop() {
            if let Some(fetched) = articles.get(&node.message_id) {
                if let Some(ref mut article) = node.article {
                    article.body = fetched.body.clone();
                }
            }
            for reply in &mut node.replies {
                stack.push(reply);
            }
        }
    }

    /// Flatten the thread tree into a list for non-recursive rendering.
    /// Uses iteration instead of recursion to avoid stack overflow.
    pub fn flatten(&self, collapse_threshold: usize) -> Vec<FlatComment> {
        let mut result = Vec::new();
        // Stack of (node, depth)
        let mut stack: Vec<(&ThreadNodeView, usize)> = vec![(self, 0)];

        while let Some((node, depth)) = stack.pop() {
            let descendant_count = Self::count_descendants(node);
            let starts_collapsed = depth >= collapse_threshold && !node.replies.is_empty();

            result.push(FlatComment {
                message_id: node.message_id.clone(),
                article: node.article.clone(),
                depth,
                descendant_count,
                starts_collapsed,
            });

            // Add replies in reverse order so they're processed in correct order
            for reply in node.replies.iter().rev() {
                stack.push((reply, depth + 1));
            }
        }

        result
    }

    /// Count total descendants of a node (iteratively)
    fn count_descendants(node: &ThreadNodeView) -> usize {
        let mut count = 0;
        let mut stack: Vec<&ThreadNodeView> = node.replies.iter().collect();

        while let Some(n) = stack.pop() {
            count += 1;
            for reply in &n.replies {
                stack.push(reply);
            }
        }

        count
    }
}

/// View model for an article
#[derive(Debug, Clone, Serialize)]
pub struct ArticleView {
    pub message_id: String,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub body: Option<String>,
    /// Raw headers for full header display (only populated for single article view)
    pub headers: Option<String>,
}

/// View model for a newsgroup
#[derive(Debug, Clone, Serialize)]
pub struct GroupView {
    pub name: String,
    pub description: Option<String>,
    pub article_count: Option<u64>,
}

/// View model for a node in the group tree hierarchy
#[derive(Debug, Clone, Serialize)]
pub struct GroupTreeNode {
    /// The segment name (e.g., "comp" or "lang" or "python")
    pub segment: String,
    /// Full group name if this is an actual group
    pub full_name: Option<String>,
    /// Group description (if this is an actual group)
    pub description: Option<String>,
    /// Child nodes (sorted alphabetically by segment)
    pub children: Vec<GroupTreeNode>,
}

impl GroupTreeNode {
    /// Build a tree from a list of groups
    pub fn build_tree(groups: &[GroupView]) -> Vec<GroupTreeNode> {
        let mut root_children: Vec<GroupTreeNode> = Vec::new();

        // Sort groups alphabetically
        let mut sorted_groups: Vec<&GroupView> = groups.iter().collect();
        sorted_groups.sort_by(|a, b| a.name.cmp(&b.name));

        for group in sorted_groups {
            let parts: Vec<&str> = group.name.split('.').collect();
            Self::insert_path(&mut root_children, &parts, &group.name, &group.description);
        }

        root_children
    }

    fn insert_path(nodes: &mut Vec<GroupTreeNode>, parts: &[&str], full_name: &str, description: &Option<String>) {
        if parts.is_empty() {
            return;
        }

        let segment = parts[0];
        let remaining = &parts[1..];

        // Find or create the node for this segment
        let node_idx = nodes.iter().position(|n| n.segment == segment);

        let node = if let Some(idx) = node_idx {
            &mut nodes[idx]
        } else {
            nodes.push(GroupTreeNode {
                segment: segment.to_string(),
                full_name: None,
                description: None,
                children: Vec::new(),
            });
            nodes.last_mut().unwrap()
        };

        if remaining.is_empty() {
            // This is a leaf node - an actual group
            node.full_name = Some(full_name.to_string());
            node.description = description.clone();
        } else {
            // Continue down the tree
            Self::insert_path(&mut node.children, remaining, full_name, description);
        }
    }

    /// Find children at a given path (e.g., "comp.lang" returns children of comp.lang)
    pub fn find_children_at_path(roots: &[GroupTreeNode], path: &str) -> Option<Vec<GroupTreeNode>> {
        if path.is_empty() {
            return Some(roots.to_vec());
        }

        let parts: Vec<&str> = path.split('.').collect();
        let mut current = roots;

        for part in &parts {
            let found = current.iter().find(|n| n.segment == *part)?;
            current = &found.children;
        }

        Some(current.to_vec())
    }

    /// Check if this node or any path prefix is an actual group
    pub fn find_node_at_path(roots: &[GroupTreeNode], path: &str) -> Option<GroupTreeNode> {
        if path.is_empty() {
            return None;
        }

        let parts: Vec<&str> = path.split('.').collect();
        let mut current = roots;

        for (i, part) in parts.iter().enumerate() {
            let found = current.iter().find(|n| n.segment == *part)?;
            if i == parts.len() - 1 {
                return Some(found.clone());
            }
            current = &found.children;
        }

        None
    }
}

impl From<&Thread> for ThreadView {
    fn from(thread: &Thread) -> Self {
        let root_view = ThreadNodeView::from(thread.root());
        let last_post_date = find_latest_date(&root_view);
        ThreadView {
            subject: thread.subject().to_string(),
            root_message_id: thread.root_message_id().to_string(),
            article_count: thread.article_count(),
            root: root_view,
            last_post_date,
        }
    }
}

/// Find the most recent date in the thread tree (iteratively)
fn find_latest_date(root: &ThreadNodeView) -> Option<String> {
    let mut latest: Option<String> = None;
    let mut stack: Vec<&ThreadNodeView> = vec![root];

    while let Some(node) = stack.pop() {
        if let Some(ref article) = node.article {
            // Compare dates - assuming RFC 2822 format, lexicographic comparison works
            // for dates in the same timezone, but we'll keep the string for now
            if latest.is_none() || article.date > *latest.as_ref().unwrap() {
                latest = Some(article.date.clone());
            }
        }
        for reply in &node.replies {
            stack.push(reply);
        }
    }

    latest
}

impl From<&ThreadNode> for ThreadNodeView {
    fn from(node: &ThreadNode) -> Self {
        thread_node_to_view(node)
    }
}

/// Convert ThreadNode to ThreadNodeView iteratively (no depth limit).
/// We use iteration instead of recursion to handle arbitrarily deep threads.
fn thread_node_to_view(root: &ThreadNode) -> ThreadNodeView {
    // Stack: (source node, built replies for parent)
    let mut stack: Vec<(&ThreadNode, Vec<ThreadNodeView>)> = vec![(root, Vec::new())];
    let mut result: Option<ThreadNodeView> = None;

    while let Some((node, built_replies)) = stack.pop() {
        let children_to_process = node.replies.len();

        if built_replies.len() == children_to_process {
            // All children processed, create this node
            let new_node = ThreadNodeView {
                message_id: node.message_id.clone(),
                article: node.article.as_ref().map(ArticleView::from),
                replies: built_replies,
            };

            // Add to parent's replies or set as result
            if let Some((parent_node, mut parent_replies)) = stack.pop() {
                parent_replies.push(new_node);
                stack.push((parent_node, parent_replies));
            } else {
                result = Some(new_node);
            }
        } else {
            // More children to process
            let next_idx = built_replies.len();
            stack.push((node, built_replies));
            stack.push((&node.replies[next_idx], Vec::new()));
        }
    }

    result.unwrap_or_else(|| ThreadNodeView {
        message_id: root.message_id.clone(),
        article: root.article.as_ref().map(ArticleView::from),
        replies: Vec::new(),
    })
}

impl From<&ThreadedArticleRef> for ArticleView {
    fn from(article: &ThreadedArticleRef) -> Self {
        ArticleView {
            message_id: article.message_id.clone(),
            subject: article.subject.clone(),
            from: article.from.clone(),
            // date is a String field
            date: article.date.clone(),
            body: None,
            headers: None,
        }
    }
}

impl From<&FetchedArticle> for ArticleView {
    fn from(article: &FetchedArticle) -> Self {
        // Extract raw headers from the article content
        let headers = extract_headers(article.raw_content());

        ArticleView {
            message_id: article.message_id().to_string(),
            subject: article.subject().to_string(),
            from: article.from().to_string(),
            date: article.date().to_string(),
            // body_text() returns Option<String>
            body: article.body_text(),
            headers,
        }
    }
}

/// Extract headers from raw article content (everything before the blank line)
fn extract_headers(content: &[u8]) -> Option<String> {
    let content_str = String::from_utf8_lossy(content);

    // Find the blank line that separates headers from body
    // NNTP uses CRLF, but we handle both cases
    let header_end = content_str
        .find("\r\n\r\n")
        .or_else(|| content_str.find("\n\n"))?;

    Some(content_str[..header_end].to_string())
}

/// Convert a ThreadCollection to a Vec of ThreadViews
pub fn threads_to_views(threads: &ThreadCollection) -> Vec<ThreadView> {
    threads.iter().map(ThreadView::from).collect()
}
