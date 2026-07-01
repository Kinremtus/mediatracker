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

    // Глобальный fallback для delete-кнопки в drawer (на случай если
    // inline hx-on:htmx:after-request не сработает из-за очередности process/insert)
    document.body.addEventListener('htmx:afterRequest', function(e) {
        const target = e.detail.target;
        if (target && target.classList && target.classList.contains('drawer-action-btn')
            && target.classList.contains('delete') && e.detail.successful) {
            console.log('[drawer] delete success, closing drawer');
            afterDelete();
        }
    });

    // Глобальный fallback для status-chips в drawer: на случай если inline
    // hx-on:htmx:after-request упадёт (e.g. event vs e), переключаем active-класс здесь
    document.body.addEventListener('htmx:afterRequest', function(e) {
        const target = e.detail.target;
        if (!target || !target.classList || !e.detail.successful) return;
        if (target.classList.contains('drawer-status-chip')) {
            const newStatus = target.classList.contains('planned') ? 'planned'
                : target.classList.contains('in_progress') ? 'in_progress'
                : target.classList.contains('completed') ? 'completed'
                : target.classList.contains('dropped') ? 'dropped' : null;
            if (newStatus) afterStatusChange(target, newStatus);
        } else if (target.classList.contains('drawer-btn-increment')) {
            const btn = target;
            const row = btn.closest('.drawer-progress-row');
            if (row) {
                const text = row.querySelector('.drawer-progress-text');
                if (text) {
                    const m = text.textContent.match(/^(\d+)/);
                    if (m) {
                        const next = parseInt(m[1]) + 1;
                        text.textContent = text.textContent.replace(/^\d+/, next);
                        const totalMatch = text.textContent.match(/\/\s*(\d+)/);
                        if (totalMatch && next >= parseInt(totalMatch[1])) btn.remove();
                    }
                }
            }
        }
    });

    // Episode checkbox toggle → server pushes HX-Trigger:
    //   {"progressUpdated": {"maxWatched": N, "mediaId": "<uuid>"},
    //    "episodesChanged": {"states": [[n, watched], ...]}}
    // Update the drawer's "X / Y эп." text, sync every visible checkbox
    // in the drawer to authoritative server state, and patch any tracking
    // card for the same media — all without a refresh.
    document.body.addEventListener('progressUpdated', function(e) {
        const maxWatched = e.detail && e.detail.maxWatched;
        const maxRead = e.detail && e.detail.maxRead;
        const progressValue = maxWatched != null ? maxWatched : maxRead;
        if (progressValue == null) return;
        const text = document.querySelector('.drawer-progress-text');
        if (text) {
            text.textContent = text.textContent.replace(/^\s*\d+/, String(progressValue));
        }
        const plusBtn = document.querySelector('.drawer-btn-increment');
        if (plusBtn) {
            const newCurrent = progressValue;
            const totalMatch = text && text.textContent.match(/\/\s*(\d+)/);
            const total = totalMatch ? parseInt(totalMatch[1]) : null;
            if (total != null && newCurrent >= total) {
                plusBtn.remove();
            } else {
                plusBtn.setAttribute('hx-vals', JSON.stringify({ progress: newCurrent + 1 }));
                // Keep the rendered +N label in sync by re-rendering? Alpine won't see hx-vals changes
                // automatically, but the label is rendered server-side on drawer load. Live with the
                // small drift (next click will re-sync via htmx:afterRequest).
            }
        }
        // Also patch any matching tracking card on the /tracking list.
        // mediaId is set when the toggle came from the drawer for a media
        // the user is tracking; cards match via data-media-id on the root.
        const mediaId = e.detail && e.detail.mediaId;
        if (mediaId) {
            const valueEl = document.querySelector(
                '.tracking-card[data-media-id="' + CSS.escape(mediaId) + '"] .tracking-progress-value'
            );
            if (valueEl) valueEl.textContent = String(progressValue);
        }
    });

    // Server pushes authoritative per-episode state after every toggle.
    // bulk-fill on watch flips 1..N rows; the HTMX response only swaps
    // the clicked row's HTML, so the other checkboxes stay stale until
    // we patch them here. states is an array of [episode_number, watched].
    document.body.addEventListener('episodesChanged', function(e) {
        const states = e.detail && e.detail.states;
        if (!Array.isArray(states)) return;
        // Only patch episodes currently rendered in the drawer/list.
        // EpisodeItem root has data-episode-n="<n>".
        const items = document.querySelectorAll('.episode-item[data-episode-n]');
        if (items.length === 0) return;
        // Build a lookup of visible episode numbers to avoid touching
        // episodes that aren't in the DOM (e.g. collapsed sections).
        const stateMap = new Map();
        for (let i = 0; i < states.length; i++) {
            const s = states[i];
            if (Array.isArray(s) && s.length >= 2) {
                stateMap.set(Number(s[0]), Boolean(s[1]));
            }
        }
        items.forEach(function(item) {
            const n = Number(item.getAttribute('data-episode-n'));
            if (!stateMap.has(n)) return;
            const watched = stateMap.get(n);
            // Toggle the row's "watched" class.
            if (watched) item.classList.add('episode-item--watched');
            else item.classList.remove('episode-item--watched');
            // Toggle the checkbox's class + glyph + title.
            const btn = item.querySelector('.episode-item-checkbox');
            if (btn) {
                if (watched) btn.classList.add('episode-item-checkbox--checked');
                else btn.classList.remove('episode-item-checkbox--checked');
                btn.textContent = watched ? '\u2713' : '\u25CB';
                btn.setAttribute(
                    'title',
                    watched ? 'Снять отметку' : 'Отметить просмотренным'
                );
            }
            // Flip the form's hx-vals so the next click sends the inverse.
            // htmx reads hx-vals on the next request, so just rewriting the
            // attribute is enough — no need to call htmx.process.
            const form = item.querySelector('form.episode-item-form');
            if (form) {
                form.setAttribute(
                    'hx-vals',
                    JSON.stringify({ watched: !watched })
                );
            }
        });
    });

    // Chapter checkbox toggle — mirror of episodesChanged but for manga.
    // Server pushes {"chaptersChanged": {"states": [[chapter_number, read], ...]}}
    document.body.addEventListener('chaptersChanged', function(e) {
        const states = e.detail && e.detail.states;
        if (!Array.isArray(states)) return;
        const items = document.querySelectorAll('.chapter-item[data-chapter-n]');
        if (items.length === 0) return;
        const stateMap = new Map();
        for (let i = 0; i < states.length; i++) {
            const s = states[i];
            if (Array.isArray(s) && s.length >= 2) {
                stateMap.set(Number(s[0]), Boolean(s[1]));
            }
        }
        items.forEach(function(item) {
            const n = Number(item.getAttribute('data-chapter-n'));
            if (!stateMap.has(n)) return;
            const read = stateMap.get(n);
            if (read) item.classList.add('chapter-item--read');
            else item.classList.remove('chapter-item--read');
            const btn = item.querySelector('.chapter-item-checkbox');
            if (btn) {
                if (read) btn.classList.add('chapter-item-checkbox--checked');
                else btn.classList.remove('chapter-item-checkbox--checked');
                btn.textContent = read ? '\u2713' : '\u25CB';
                btn.setAttribute(
                    'title',
                    read ? 'Снять отметку' : 'Отметить прочитанным'
                );
            }
            const form = item.querySelector('form.chapter-item-form');
            if (form) {
                form.setAttribute(
                    'hx-vals',
                    JSON.stringify({ read: !read })
                );
            }
        });
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

    // Update page title and filter button label when tracking grid is swapped
    const statusLabels = {
        '': 'Все списки',
        'in_progress': 'В процессе',
        'completed': 'Завершено',
        'planned': 'Запланировано',
        'dropped': 'Брошено'
    };
    function updateFilterUI() {
        const params = new URLSearchParams(window.location.search);
        const status = params.get('status') || '';
        const mediaType = params.get('type') || '';
        const label = statusLabels[status] || status;

        // Update page title
        const title = document.querySelector('.content-title');
        if (title) title.textContent = label;

        // Update status filter button text
        const filterBtns = document.querySelectorAll('.filter-btn');
        if (filterBtns.length >= 2) {
            const btn = filterBtns[1];
            const arrow = btn.querySelector('span');
            btn.textContent = label + ' ';
            if (arrow) btn.appendChild(arrow);
        }

        // Update active states in status dropdown
        const statusItems = document.querySelectorAll('.filter-dropdown-menu:last-child .filter-dropdown-item');
        statusItems.forEach(item => {
            const href = item.getAttribute('href') || '';
            const itemStatus = new URLSearchParams(href.split('?')[1] || '').get('status') || '';
            item.classList.toggle('active', itemStatus === status);
        });
    }
    document.body.addEventListener('htmx:afterSwap', function(e) {
        if (e.detail.target && e.detail.target.id === 'tracking-grid') {
            updateFilterUI();
        }
    });

    // Close filter dropdowns when a filter request is about to fire
    document.body.addEventListener('htmx:beforeRequest', function(e) {
        const target = e.detail && e.detail.target;
        if (target && target.id === 'tracking-grid') {
            const filterBar = document.querySelector('.filter-bar');
            if (filterBar && typeof Alpine !== 'undefined') {
                const data = Alpine.$data(filterBar);
                if (data) { data.typeOpen = false; data.statusOpen = false; }
            }
        }
    });

    // Remove card from filtered list if its status no longer matches the filter
    document.body.addEventListener('htmx:afterSwap', function(e) {
        const target = e.detail.target;
        if (!target || !target.classList || !target.classList.contains('tracking-card')) return;

        const currentStatus = new URLSearchParams(window.location.search).get('status');
        if (!currentStatus) return;

        // After outerHTML swap, e.detail.target is the OLD element (detached).
        // Find the NEW card in DOM by the same ID.
        const newCard = document.getElementById(target.id);
        if (!newCard) return;

        const cardStatus = newCard.getAttribute('data-status');
        if (cardStatus && cardStatus !== currentStatus) {
            newCard.remove();
            // Check if grid is now empty
            const grid = document.querySelector('.tracking-grid');
            if (grid && grid.children.length === 0) {
                grid.outerHTML = '<div class="empty-state"><div class="empty-state-icon"></div><p class="empty-state-text">Список пуст</p><p class="empty-state-sub">Добавьте тайтлы через <a href="/search" style="color: var(--in_progress);">поиск</a></p></div>';
            }
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
            // x-html injects raw HTML — HTMX won't process it automatically
            setTimeout(() => {
                const el = document.querySelector('.media-drawer-content');
                if (el && typeof htmx !== 'undefined') htmx.process(el);
            }, 0);
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
    console.log('[drawer] afterDelete called');
    // Close drawer
    const appShell = document.querySelector('.app-shell');
    if (!appShell) {
        console.warn('[drawer] .app-shell not found');
        return;
    }
    const data = Alpine.$data(appShell);
    if (!data) {
        console.warn('[drawer] Alpine data not found on .app-shell');
        return;
    }
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

// --- Drawer: after add to tracking — refetch content to show controls ---
function afterAddToTracking(event, form) {
    const provider = form.querySelector('input[name="provider"]').value;
    const externalId = form.querySelector('input[name="external_id"]').value;
    fetch(`/api/media/${provider}/${externalId}`)
        .then(r => r.text())
        .then(html => {
            const appShell = document.querySelector('.app-shell');
            const data = Alpine.$data(appShell);
            data.drawerContent = html;
            setTimeout(() => {
                const el = document.querySelector('.media-drawer-content');
                if (el && typeof htmx !== 'undefined') htmx.process(el);
            }, 0);
        });
    refreshTrackingList();
}

// --- Refresh Tracking List (after delete from drawer) ---
function refreshTrackingList() {
    if (typeof htmx !== 'undefined') {
        const grid = document.querySelector('.tracking-grid');
        if (grid) {
            const params = new URLSearchParams(window.location.search);
            const url = '/tracking/partial' + (params.toString() ? '?' + params.toString() : '');
            htmx.ajax('GET', url, {target: grid, swap: 'innerHTML'});
        }
    }
}

// --- Save Recent Media (for quick search) ---
function saveRecentMedia(item) {
    let recent = JSON.parse(localStorage.getItem('recentMedia') || '[]');
    const key = item.provider + ':' + item.external_id;
    recent = recent.filter(i => i.provider + ':' + i.external_id !== key);
    recent.unshift({
        provider: item.provider,
        external_id: item.external_id,
        media_type: item.media_type,
        title: item.title || item.media_type,
        poster_url: item.poster_url || null,
        score: item.score || null,
        year: item.year || null,
    });
    localStorage.setItem('recentMedia', JSON.stringify(recent.slice(0, 5)));
}

function saveRecentMediaFromElement(el) {
    saveRecentMedia({
        provider: el.dataset.provider,
        external_id: el.dataset.externalId,
        media_type: el.dataset.mediaType,
        title: el.dataset.title,
        poster_url: el.dataset.posterUrl || null,
        score: parseFloat(el.dataset.score) || null,
        year: parseInt(el.dataset.year) || null,
    });
}

// --- Search Overlay (Alpine component) ---
// Listens for 'open-search' custom event dispatched from header triggers.
function searchOverlay() {
    return {
        open: false,
        query: '',
        results: [],
        loading: false,
        recentMedia: JSON.parse(localStorage.getItem('recentMedia') || '[]'),

        init() {
            window.addEventListener('open-search', () => this.openSearch());
        },

        openSearch() {
            this.open = true;
            this.recentMedia = JSON.parse(localStorage.getItem('recentMedia') || '[]');
            this.$nextTick(() => {
                this.$refs.overlayInput?.focus();
            });
        },

        closeSearch() {
            this.open = false;
            this.query = '';
            this.results = [];
        },

        async search() {
            if (this.query.length < 2) {
                this.results = [];
                return;
            }
            this.loading = true;
            try {
                const resp = await fetch('/api/search/suggestions?q=' + encodeURIComponent(this.query));
                if (resp.ok) {
                    this.results = await resp.json();
                }
            } catch (e) {
                this.results = [];
            } finally {
                this.loading = false;
            }
        },

        submitSearch() {
            const q = this.query.trim();
            if (q) {
                window.location.href = '/search?q=' + encodeURIComponent(q);
            }
        },

        clickResult(item) {
            this.closeSearch();
            saveRecentMedia(item);
            openMediaDrawer(item.provider, item.external_id, item.media_type);
        },
    };
}
