// ============================================
// MediaTracker — Custom JS (HTMX + Alpine)
// ============================================

// --- HTMX Configuration ---
document.addEventListener('DOMContentLoaded', function() {
    // Show loading indicator (skip for hx-swap="none" elements like drawer buttons)
    document.body.addEventListener('htmx:beforeRequest', function(e) {
        const target = e.detail.target;
        if (target && target.getAttribute('hx-swap') !== 'none') {
            target.style.opacity = '0.6';
            target.style.pointerEvents = 'none';
        }
    });

    // Hide loading indicator
    document.body.addEventListener('htmx:afterRequest', function(e) {
        const target = e.detail.target;
        if (target && target.getAttribute('hx-swap') !== 'none') {
            target.style.opacity = '1';
            target.style.pointerEvents = '';
        }
    });

    // Flash notification on successful update
    document.body.addEventListener('htmx:oobAfterSwap', function(e) {
        if (e.detail.target.id === 'settings-messages') {
            // Auto-hide success messages after 3 seconds
            const successMsg = e.detail.target.querySelector('.settings-message-success');
            if (successMsg) {
                setTimeout(function() {
                    successMsg.style.transition = 'opacity 0.3s';
                    successMsg.style.opacity = '0';
                    setTimeout(function() { successMsg.remove(); }, 300);
                }, 3000);
            }
        }
    });

    // Handle delete - remove card from DOM
    document.body.addEventListener('htmx:afterSwap', function(e) {
        if (e.detail.xhr.status === 200 && e.detail.target.classList && e.detail.target.classList.contains('tracking-card')) {
            // Card was deleted - it's now empty
            const card = e.detail.target;
            if (card.innerHTML.trim() === '') {
                card.remove();
                // Check if grid is empty
                const grid = document.querySelector('.tracking-grid');
                if (grid && grid.children.length === 0) {
                    grid.outerHTML = '<div class="empty-state"><div class="empty-state-icon"></div><p class="empty-state-text">Список пуст</p><p class="empty-state-sub">Добавьте тайтлы через <a href="/search" style="color: var(--in_progress);">поиск</a></p></div>';
                }
            }
        }
    });

    // Re-initialize Alpine on HTMX content swap
    document.body.addEventListener('htmx:afterSwap', function(e) {
        if (typeof Alpine !== 'undefined') {
            Alpine.initTree(e.detail.target);
        }
    });
});

// --- Theme (from base.html) ---
function setTheme(t) {
    localStorage.setItem('mediatracker-theme', t);
    document.documentElement.className = t;
}

// --- Media Drawer (Alpine integration) ---
function openMediaDrawer(provider, externalId, mediaType) {
    let url = `/api/media/${provider}/${externalId}`;
    if (mediaType) url += `?media_type=${mediaType}`;
    fetch(url)
        .then(r => r.text())
        .then(html => {
            const appShell = document.querySelector('.app-shell');
            const data = Alpine.$data(appShell);
            data.drawerContent = html;
            data.drawerOpen = true;
            document.body.style.overflow = 'hidden';
        })
        .catch(e => console.error('Failed to load media details', e));
}

// --- Drawer: HTMX callbacks after status/progress change ---
function afterStatusChange(btn, newStatus) {
    // Update active chip in drawer
    const container = btn.closest('.drawer-status-chips');
    if (container) {
        container.querySelectorAll('.drawer-status-chip').forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
    }
    // Refresh tracking grid on the page (if it exists)
    refreshTrackingList();
}

function afterProgressIncrement(btn, newProgress) {
    // Update progress text in drawer
    const row = btn.closest('.drawer-progress-row');
    if (row) {
        const text = row.querySelector('.drawer-progress-text');
        if (text) {
            const current = text.textContent.trim();
            text.textContent = current.replace(/\d+/, newProgress);
        }
        // Hide button if reached total
        const tcMatch = text.textContent.match(/\/\s*(\d+)/);
        if (tcMatch && newProgress >= parseInt(tcMatch[1])) {
            btn.remove();
        }
    }
    // Refresh tracking grid on the page
    refreshTrackingList();
}

function afterDelete() {
    // Close drawer
    const appShell = document.querySelector('.app-shell');
    const data = Alpine.$data(appShell);
    data.drawerOpen = false;
    document.body.style.overflow = '';
    // Refresh tracking grid
    refreshTrackingList();
}

// --- Rating (half-star precision) ---
function setRatingHalf(trackingId, starIndex, event) {
    const rect = event.target.getBoundingClientRect();
    const clickX = event.clientX - rect.left;
    const isLeftHalf = clickX < rect.width / 2;
    const rating = isLeftHalf ? starIndex - 0.5 : starIndex;

    const params = new URLSearchParams();
    params.append('rating', rating);
    fetch(`/tracking/${trackingId}/htmx`, {
        method: 'POST',
        body: params,
    }).then(() => {
        // Update star display
        updateStarsDisplay(rating);
        // Update rating value text
        const valEl = document.querySelector('.drawer-rating-value');
        if (valEl) valEl.textContent = rating.toFixed(1) + '/10';
    }).catch(() => {});
}

function updateStarsDisplay(rating) {
    const stars = document.querySelectorAll('.drawer-stars .drawer-star');
    stars.forEach((star, idx) => {
        const starNum = idx + 1;
        star.classList.remove('active', 'half');
        if (rating >= starNum) {
            star.classList.add('active');
        } else if (rating >= starNum - 0.5) {
            star.classList.add('half');
        }
    });
}

function previewRating(star, index) {
    const stars = document.querySelectorAll('.drawer-stars .drawer-star');
    stars.forEach((s, idx) => {
        if (idx < index - 1) s.style.color = '#facc15';
        else if (idx === index - 1) s.style.color = '#facc15';
    });
}

function resetRatingPreview() {
    // Re-render based on current rating
    const valEl = document.querySelector('.drawer-rating-value');
    if (valEl) {
        const text = valEl.textContent.replace('/10', '');
        if (text !== '—') {
            updateStarsDisplay(parseFloat(text));
        }
    }
    document.querySelectorAll('.drawer-stars .drawer-star').forEach(s => s.style.color = '');
}

// --- Refresh Tracking List (after delete from drawer) ---
function refreshTrackingList() {
    if (typeof htmx !== 'undefined') {
        const grid = document.querySelector('.tracking-grid');
        if (grid) {
            htmx.ajax('GET', '/tracking/partial', {target: grid, swap: 'innerHTML'});
        }
    }
}
