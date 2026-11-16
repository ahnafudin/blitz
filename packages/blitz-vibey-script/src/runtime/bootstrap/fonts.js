    // CSS-declared faces become loaded only after fetch, decode, registration and layout.
    const faceRecords = new Map();
    const fontWaiters = [];
    let fontReady;
    let fontReadyResolve;
    let fontReadyDone = false;
    let fontsLoading = false;
    function resetFontReady() {
        fontReadyDone = false;
        fontReady = new Promise(resolve => { fontReadyResolve = resolve; });
    }
    resetFontReady();
    function fontError() { return new DOMException('The declared font could not be loaded', 'NetworkError'); }
    function awaitFaces(ids) {
        const records = ids.map(id => faceRecords.get(id));
        if (records.some(record => !record || record.status === 'error')) return Promise.reject(fontError());
        if (records.every(record => record.status === 'loaded')) return Promise.resolve(records.map(record => record.face));
        return new Promise((resolve, reject) => { fontWaiters.push({ ids, resolve, reject }); });
    }
    function refreshFonts(requested) {
        const waiting = fontWaiters.filter(waiter => waiter.font);
        const snapshot = __blitz_font_snapshot(requested, waiting.map(waiter => waiter.font));
        const waitingIds = new Map(waiting.map((waiter, index) => [waiter, snapshot.waiterIds[index]]));
        const present = new Set();
        for (const data of snapshot.faces) {
            present.add(data.id);
            let record = faceRecords.get(data.id);
            if (!record) {
                record = { ...data, loaded: null };
                const face = {};
                for (const key of ['family', 'status', 'style', 'weight']) {
                    Object.defineProperty(face, key, { enumerable: true, get: () => String(record[key]) });
                }
                Object.defineProperty(face, 'loaded', { get: () => record.loaded ||= awaitFaces([record.id]).then(() => face) });
                face.load = () => face.loaded;
                record.face = face;
                faceRecords.set(data.id, record);
            }
            record.status = data.status;
        }
        for (const id of faceRecords.keys()) if (!present.has(id)) faceRecords.delete(id);
        const pending = snapshot.blocked || snapshot.faces.some(face => face.status === 'loading');
        let changed = false;
        if (pending && !fontsLoading) {
            if (fontReadyDone) resetFontReady();
            fontsLoading = true;
        }
        for (let i = fontWaiters.length - 1; i >= 0; i--) {
            const waiter = fontWaiters[i];
            if (waiter.font) {
                if (snapshot.blocked) continue;
                waiter.ids = waitingIds.get(waiter);
                delete waiter.font;
            }
            const records = waiter.ids.map(id => faceRecords.get(id));
            if (records.some(record => !record || record.status === 'error')) {
                fontWaiters.splice(i, 1); waiter.reject(fontError()); changed = true;
            } else if (records.every(record => record.status === 'loaded')) {
                fontWaiters.splice(i, 1); waiter.resolve(records.map(record => record.face)); changed = true;
            }
        }
        if (!pending && !fontReadyDone) {
            fontsLoading = false; fontReadyDone = true; fontReadyResolve(fontFaceSet); changed = true;
        }
        return { ...snapshot, changed };
    }
    const fontFaceSet = {
        get status() { refreshFonts(); return fontsLoading ? 'loading' : 'loaded'; },
        get size() { refreshFonts(); return faceRecords.size; },
        get ready() { refreshFonts(); return fontReady; },
        check(font) {
            const snapshot = refreshFonts(String(font));
            return !snapshot.blocked && snapshot.ids.every(id => faceRecords.get(id)?.status === 'loaded');
        },
        load(font) {
            try {
                const requested = String(font);
                const snapshot = refreshFonts(requested);
                if (snapshot.blocked) return new Promise((resolve, reject) => { fontWaiters.push({ font: requested, ids: [], resolve, reject }); });
                return awaitFaces(snapshot.ids);
            } catch (error) { return Promise.reject(error); }
        },
        forEach(callback, thisArg) { for (const face of this.values()) callback.call(thisArg, face, face, this); },
        *values() { refreshFonts(); for (const record of faceRecords.values()) yield record.face; },
        keys() { return this.values(); },
        *entries() { for (const face of this.values()) yield [face, face]; },
        [Symbol.iterator]() { return this.values(); },
    };
    document.fonts = fontFaceSet;
    globalThis.__blitz_poll_fonts = () => refreshFonts().changed;
