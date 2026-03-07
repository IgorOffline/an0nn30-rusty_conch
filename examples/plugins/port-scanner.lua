-- plugin-name: Port Scanner
-- plugin-description: TCP port scanner with common port detection
-- plugin-type: panel
-- plugin-version: 1.0.0
-- plugin-icon: port-scanner-icon.png
-- plugin-keybind: open_panel = cmd+shift+o | Toggle Port Scanner panel

-- Well-known ports and their service names
local KNOWN_PORTS = {
    [21]   = "FTP",
    [22]   = "SSH",
    [23]   = "Telnet",
    [25]   = "SMTP",
    [53]   = "DNS",
    [80]   = "HTTP",
    [110]  = "POP3",
    [111]  = "RPC",
    [135]  = "MSRPC",
    [139]  = "NetBIOS",
    [143]  = "IMAP",
    [443]  = "HTTPS",
    [445]  = "SMB",
    [993]  = "IMAPS",
    [995]  = "POP3S",
    [1433] = "MSSQL",
    [1521] = "Oracle",
    [2049] = "NFS",
    [3306] = "MySQL",
    [3389] = "RDP",
    [5432] = "PostgreSQL",
    [5900] = "VNC",
    [6379] = "Redis",
    [8080] = "HTTP-Alt",
    [8443] = "HTTPS-Alt",
    [9200] = "Elasticsearch",
    [9090] = "Prometheus",
    [11211] = "Memcached",
    [27017] = "MongoDB",
}

-- Commonly scanned port list (sorted)
local COMMON_PORTS = {
    21, 22, 23, 25, 53, 80, 110, 111, 135, 139, 143,
    443, 445, 993, 995, 1433, 1521, 2049, 3306, 3389,
    5432, 5900, 6379, 8080, 8443, 9090, 9200, 11211, 27017,
}

-- State
local scan_target = ""
local scan_results = nil
local scan_time = nil
local is_scanning = false

-- Cached server list (name -> host)
local server_list = {}  -- array of {name=, host=}

function setup()
    -- Load configured servers
    local details = app.server_details()
    if details then
        server_list = details
    end

    -- Try to get the host from the current session
    local info = session.current()
    if info and info.type == "ssh" then
        local title = info.title or ""
        local host = title:match("@([%w%.%-]+)") or title:match("([%w%.%-]+)")
        if host then
            scan_target = host
        end
    end
end

function render()
    ui.panel_clear()

    ui.panel_heading("Port Scanner")

    if is_scanning then
        ui.panel_label("Scanning " .. scan_target .. "...")
        ui.panel_progress("Scan", 0.5, "in progress...")
        return
    end

    -- Target input hint
    if scan_target ~= "" then
        ui.panel_kv("Target:", scan_target)
    else
        ui.panel_label("No target set. Click 'Configure Scan' to begin.")
    end

    ui.panel_separator()

    -- Controls
    ui.panel_button("configure", "Configure Scan")

    if scan_target ~= "" then
        ui.panel_button("scan_common", "Quick Scan (Common Ports)")
        ui.panel_button("scan_range", "Range Scan (1-1024)")
        ui.panel_button("scan_all_known", "Scan All Known Services")
    end

    -- Results
    if scan_results then
        ui.panel_separator()
        ui.panel_heading("Results")

        if scan_time then
            ui.panel_kv("Scan time:", scan_time)
        end

        local open_count = 0
        for _, r in ipairs(scan_results) do
            if r.open then open_count = open_count + 1 end
        end

        ui.panel_kv("Open ports:", tostring(open_count))
        ui.panel_kv("Scanned:", tostring(#scan_results) .. " ports")

        if open_count > 0 then
            ui.panel_separator()
            ui.panel_heading("Open Ports")

            local columns = {"Port", "Service", "State"}
            local rows = {}
            for _, r in ipairs(scan_results) do
                if r.open then
                    local service = KNOWN_PORTS[r.port] or "unknown"
                    table.insert(rows, {tostring(r.port), service, "open"})
                end
            end
            ui.panel_table(columns, rows)
        else
            ui.panel_separator()
            ui.panel_label("No open ports found.")
        end

        ui.panel_separator()
        ui.panel_button("export", "Copy Results")
    end
end

function on_click(id)
    if id == "configure" then
        do_configure()
    elseif id == "scan_common" then
        do_scan("common")
    elseif id == "scan_range" then
        do_scan("range")
    elseif id == "scan_all_known" then
        do_scan("known")
    elseif id == "export" then
        do_export()
    end
end

function do_configure()
    -- Build dropdown options: "Custom Host" + each configured server
    local options = { "Custom Host" }
    local host_map = {}  -- option label -> actual host
    for _, srv in ipairs(server_list) do
        local label = srv.name .. " (" .. srv.host .. ")"
        table.insert(options, label)
        host_map[label] = srv.host
    end

    -- Figure out current default selection
    local default_selection = "Custom Host"
    for _, srv in ipairs(server_list) do
        if srv.host == scan_target then
            default_selection = srv.name .. " (" .. srv.host .. ")"
            break
        end
    end

    local default_host = scan_target
    if default_host == "" then
        default_host = "localhost"
    end

    local vals = ui.form("Port Scanner Configuration", {
        { type = "combo", name = "source", label = "Scan Target", options = options, default = default_selection },
        { type = "separator" },
        { type = "text",  name = "custom_host", label = "Custom Host (if selected above)", default = default_host },
        { type = "separator" },
        { type = "label", text = "Quick scan checks ~30 common ports." },
        { type = "label", text = "Range scan checks ports 1-1024." },
        { type = "label", text = "You can also scan all known service ports." },
    })

    if vals then
        local source = vals.source or "Custom Host"
        if source == "Custom Host" then
            local host = vals.custom_host or ""
            if host ~= "" then
                scan_target = host
                scan_results = nil
                scan_time = nil
            end
        else
            -- Look up the actual host for the selected server
            local host = host_map[source]
            if host and host ~= "" then
                scan_target = host
                scan_results = nil
                scan_time = nil
            end
        end
    end
end

function do_scan(mode)
    if scan_target == "" then
        ui.error("Error", "No target host configured.")
        return
    end

    -- Resolve the host first to verify it's valid
    local ips = net.resolve(scan_target)
    if not ips or #ips == 0 then
        ui.error("DNS Error", "Could not resolve: " .. scan_target)
        return
    end

    is_scanning = true

    local start_time = net.time()
    local results

    if mode == "common" then
        results = net.scan(scan_target, COMMON_PORTS, 1500, 50)
    elseif mode == "range" then
        -- Scan 1-1024 with higher concurrency
        local ports = {}
        for i = 1, 1024 do ports[i] = i end
        results = net.scan(scan_target, ports, 1500, 100)
    elseif mode == "known" then
        -- All known service ports
        local ports = {}
        for port, _ in pairs(KNOWN_PORTS) do
            ports[#ports + 1] = port
        end
        results = net.scan(scan_target, ports, 1500, 50)
    end

    local elapsed = net.time() - start_time

    -- Sort results by port number
    if results then
        table.sort(results, function(a, b) return a.port < b.port end)
    end

    scan_results = results or {}
    scan_time = string.format("%.1fs", elapsed)
    is_scanning = false
end

function do_export()
    if not scan_results then return end

    local lines = {}
    table.insert(lines, "Port Scanner Results for " .. scan_target)
    table.insert(lines, "Scan time: " .. (scan_time or "unknown"))
    table.insert(lines, "")
    table.insert(lines, string.format("%-8s %-20s %s", "PORT", "SERVICE", "STATE"))
    table.insert(lines, string.rep("-", 40))

    for _, r in ipairs(scan_results) do
        if r.open then
            local service = KNOWN_PORTS[r.port] or "unknown"
            table.insert(lines, string.format("%-8d %-20s %s", r.port, service, "open"))
        end
    end

    local text = table.concat(lines, "\n")
    app.clipboard(text)
    ui.alert("Copied", "Scan results copied to clipboard.")
end
