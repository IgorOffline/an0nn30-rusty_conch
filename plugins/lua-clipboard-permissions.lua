-- plugin-name: Clipboard Permissions Demo
-- plugin-description: Example plugin that declares clipboard permissions and uses clipboard APIs
-- plugin-version: 0.1.0
-- plugin-type: action
-- plugin-api: ^1.0
-- plugin-permissions: ui.menu, ui.notify, clipboard.read, clipboard.write, bus.publish, bus.query, session.status

local ACTION_SHOW = "clipboard_demo_show"
local ACTION_UPPER = "clipboard_demo_upper"
local ACTION_QUERY = "clipboard_demo_query"

local function preview(text, max_len)
    if text == nil then
        return "(clipboard is empty)"
    end
    if #text <= max_len then
        return text
    end
    return string.sub(text, 1, max_len) .. "..."
end

function setup()
    app.log("info", "Clipboard Permissions Demo loaded")
    app.register_service("clipboard_demo")
    app.register_command("Clipboard: Show Preview", ACTION_SHOW)
    app.register_command("Clipboard: UPPERCASE Selection", ACTION_UPPER)
    app.register_command("Clipboard: Query Service", ACTION_QUERY)
    app.set_status("Clipboard demo ready", "info", -1.0)
end

function on_query(method, args_json)
    if method == "preview" then
        local text = app.clipboard_get()
        return string.format('{"preview":%q}', preview(text, 80))
    end
    if method == "status" then
        local sess = session.current()
        local t = (sess and sess.type) or "unknown"
        return string.format('{"session_type":%q}', t)
    end
    return "null"
end

function on_event(event)
    if type(event) ~= "table" then
        return
    end

    if event.action == ACTION_SHOW then
        local text = app.clipboard_get()
        app.notify("Clipboard Preview", preview(text, 120), "info", 3500)
        return
    end

    if event.action == ACTION_QUERY then
        local result = app.query_plugin("clipboard_demo", "status", {})
        app.notify("Clipboard Service Query", result or "nil", "info", 3500)
        return
    end

    if event.action == ACTION_UPPER then
        local text = app.clipboard_get()
        if text == nil or text == "" then
            app.notify("Clipboard", "Nothing to transform", "warn", 2500)
            return
        end
        app.clipboard(string.upper(text))
        app.notify("Clipboard", "Transformed text copied back to clipboard", "success", 2500)
        app.set_status("Clipboard transformed", "success", -1.0)
    end
end
