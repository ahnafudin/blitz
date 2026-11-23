// A content attribute and the corresponding IDL property occupy one place in the
// target's listener list. Changing its value preserves that place; clearing releases it.
(function installImageHandlers(proto, report) {
    const handlers = new WeakMap();
    const add = Node.prototype.addEventListener;
    const remove = Node.prototype.removeEventListener;
    const brand = Object.getOwnPropertyDescriptor(proto, "complete").get;
    const apply = Reflect.apply;
    const FunctionConstructor = Function;
    const ownerDocument = document;

    function current(image, type) {
        const slot = handlers.get(image)?.[type];
        if (!slot) return null;
        if (typeof slot.value === "string") {
            const source = slot.value;
            slot.value = null;
            try {
                // Parse an independent handler body first. Only a valid body can
                // enter the wrapper that gives it the HTML element/document scopes.
                FunctionConstructor("event", source);
                const factory = FunctionConstructor("doc",
                    "with(doc){with(this){return function on" + type + "(event){\n" + source + "\n};}}");
                slot.value = apply(factory, image, [ownerDocument]);
            } catch (error) {
                report(error);
            }
        }
        return slot.value;
    }

    function update(image, type, value) {
        let table = handlers.get(image);
        const old = table?.[type];
        if (value === null) {
            if (old) {
                apply(remove, image, [type, old.listener]);
                delete table[type];
            }
            return;
        }
        if (old) {
            old.value = value;
            return;
        }
        if (!table) handlers.set(image, table = Object.create(null));
        const listener = event => {
            const fn = current(image, type);
            if (fn !== null) apply(fn, image, [event]);
        };
        table[type] = { value, listener };
        apply(add, image, [type, listener]);
    }

    for (const type of ["load", "error"]) {
        Object.defineProperty(proto, "on" + type, {
            configurable: true, enumerable: true,
            get() { apply(brand, this, []); return current(this, type); },
            set(fn) { apply(brand, this, []); update(this, type, typeof fn === "function" ? fn : null); }
        });
    }
    return update;
})
