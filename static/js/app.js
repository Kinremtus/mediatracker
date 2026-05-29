// ============================================
// MediaTracker — Custom JS (HTMX + Alpine)
// ============================================

// --- HTMX Configuration ---
document.addEventListener('DOMContentLoaded', function() {
    // Show loading indicator
    document.body.addEventListener('htmx:beforeRequest', function(e) {
        const target = e.detail.target;
        if (target) {
            target.style.opacity = '0.6';
            target.style.pointerEvents = 'none';
        }
    });

    // Hide loading indicator
    document.body.addEventListener('htmx:afterRequest', function(e) {
        const target = e.detail.target;
        if (target) {
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

function updateTrackingStatus(trackingId, newStatus, clickedBtn) {
    const params = new URLSearchParams();
    params.append('status', newStatus);
    fetch(`/tracking/${trackingId}`, {
        method: 'POST',
        body: params,
        redirect: 'manual',
    }).then(() => {
        const container = clickedBtn.closest('.drawer-status-chips');
        if (container) {
            container.querySelectorAll('.drawer-status-chip').forEach(btn => btn.classList.remove('active'));
            clickedBtn.classList.add('active');
        }
        setTimeout(() => { window.location.href = window.location.href; }, 200);
    }).catch(() => {});
}
