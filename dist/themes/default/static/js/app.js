// Toggle reply form visibility
function toggleReplyForm(button) {
    var container = button.closest('.comment, .article-view');
    var formContainer = container.querySelector('.reply-form-container');
    if (formContainer) {
        if (formContainer.style.display === 'none') {
            formContainer.style.display = 'block';
            var textarea = formContainer.querySelector('textarea');
            if (textarea) textarea.focus();
        } else {
            formContainer.style.display = 'none';
        }
    }
}

document.addEventListener('DOMContentLoaded', function() {
    // Thread collapse/expand functionality for flat comment list
    var comments = document.querySelectorAll('.comment');
    var commentsArray = Array.prototype.slice.call(comments);

    // Get descendants of a comment (all following comments with greater depth)
    function getDescendants(comment, commentsArray) {
        var descendants = [];
        var commentDepth = parseInt(comment.dataset.depth, 10);
        var startIndex = commentsArray.indexOf(comment);

        for (var i = startIndex + 1; i < commentsArray.length; i++) {
            var nextDepth = parseInt(commentsArray[i].dataset.depth, 10);
            if (nextDepth > commentDepth) {
                descendants.push(commentsArray[i]);
            } else {
                break; // Reached a sibling or ancestor
            }
        }
        return descendants;
    }

    // Initialize collapsed state
    commentsArray.forEach(function(comment) {
        if (comment.dataset.collapsed === 'true') {
            var descendants = getDescendants(comment, commentsArray);
            descendants.forEach(function(desc) {
                desc.classList.add('collapsed-hidden');
            });
        }
    });

    // Handle expand/collapse buttons
    document.querySelectorAll('.expand-replies').forEach(function(button) {
        button.addEventListener('click', function() {
            var comment = this.closest('.comment');
            var isCollapsed = comment.dataset.collapsed === 'true';
            var descendants = getDescendants(comment, commentsArray);
            var count = this.dataset.count;

            if (isCollapsed) {
                // Expand: show descendants (but respect their own collapsed state)
                descendants.forEach(function(desc) {
                    desc.classList.remove('collapsed-hidden');
                    // If this descendant is itself collapsed, hide its descendants
                    if (desc.dataset.collapsed === 'true') {
                        var subDescendants = getDescendants(desc, commentsArray);
                        subDescendants.forEach(function(sub) {
                            sub.classList.add('collapsed-hidden');
                        });
                    }
                });
                comment.dataset.collapsed = 'false';
                this.textContent = 'Hide replies';
            } else {
                // Collapse: hide all descendants
                descendants.forEach(function(desc) {
                    desc.classList.add('collapsed-hidden');
                });
                comment.dataset.collapsed = 'true';
                this.textContent = 'Show ' + count + ' more replies';
            }
        });
    });

    // Make comment borders clickable to collapse/expand
    commentsArray.forEach(function(comment) {
        // Skip root level comments
        if (comment.classList.contains('depth-0')) return;

        comment.addEventListener('click', function(e) {
            // Only collapse if clicking on the border area (leftmost 10px)
            var rect = comment.getBoundingClientRect();
            if (e.clientX - rect.left < 10) {
                e.stopPropagation();
                var descendants = getDescendants(comment, commentsArray);
                if (descendants.length === 0) return;

                var isHidden = descendants[0].classList.contains('collapsed-hidden');
                if (isHidden) {
                    // Expand
                    descendants.forEach(function(desc) {
                        desc.classList.remove('collapsed-hidden');
                    });
                } else {
                    // Collapse
                    descendants.forEach(function(desc) {
                        desc.classList.add('collapsed-hidden');
                    });
                }
            }
        });
    });

    // Group search/filter functionality (home page)
    var searchInput = document.getElementById('group-search');
    var cardsView = document.getElementById('cards-view');
    var searchResults = document.getElementById('search-results');

    if (searchInput && cardsView && searchResults) {
        var cards = cardsView.querySelectorAll('.group-card');
        var resultItems = searchResults.querySelectorAll('.search-result-item');

        searchInput.addEventListener('input', function() {
            var query = this.value.trim().toLowerCase();

            if (query === '') {
                // Show cards, hide search results
                cardsView.style.display = 'flex';
                searchResults.style.display = 'none';
                cards.forEach(function(card) {
                    card.classList.remove('hidden');
                });
            } else if (query.length < 3) {
                // Filter cards by name for short queries
                cardsView.style.display = 'flex';
                searchResults.style.display = 'none';
                cards.forEach(function(card) {
                    var name = card.getAttribute('data-name').toLowerCase();
                    if (name.indexOf(query) !== -1) {
                        card.classList.remove('hidden');
                    } else {
                        card.classList.add('hidden');
                    }
                });
            } else {
                // Show full group search results for longer queries
                cardsView.style.display = 'none';
                searchResults.style.display = 'block';

                var hasResults = false;
                resultItems.forEach(function(item) {
                    var groupName = item.getAttribute('data-group').toLowerCase();
                    if (groupName.indexOf(query) !== -1) {
                        item.classList.remove('hidden');
                        hasResults = true;
                    } else {
                        item.classList.add('hidden');
                    }
                });

                // Show "no results" message if needed
                var noResultsEl = searchResults.querySelector('.no-results');
                if (!hasResults) {
                    if (!noResultsEl) {
                        noResultsEl = document.createElement('p');
                        noResultsEl.className = 'no-results';
                        noResultsEl.textContent = 'No groups found matching your search.';
                        searchResults.querySelector('.search-results-list').appendChild(noResultsEl);
                    }
                    noResultsEl.style.display = 'block';
                } else if (noResultsEl) {
                    noResultsEl.style.display = 'none';
                }
            }
        });
    }
});
