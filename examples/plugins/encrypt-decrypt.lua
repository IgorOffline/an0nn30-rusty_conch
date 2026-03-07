-- plugin-name: Encrypt / Decrypt
-- plugin-description: AES encryption (CBC, GCM, ECB) with PBKDF2 key derivation
-- plugin-version: 2.1.0
-- plugin-icon: encrypt-decrypt-icon.png
-- plugin-keybind: run = cmd+shift+y | Run Encrypt/Decrypt tool

-- Get available algorithms from the Rust crypto module
local ALGORITHMS = crypto.algorithms()

-- Main
local vals = ui.form("Encrypt / Decrypt", {
    { type = "combo",    name = "mode",      label = "Mode",       options = { "Encrypt", "Decrypt" }, default = "Encrypt" },
    { type = "combo",    name = "algorithm",  label = "Algorithm",  options = ALGORITHMS, default = "AES-256-GCM" },
    { type = "password", name = "key",        label = "Passphrase" },
    { type = "separator" },
    { type = "label",    text = "GCM is recommended (authenticated encryption)" },
    { type = "text",     name = "input",      label = "Input",      default = "" },
})

if not vals then return end

local mode      = vals.mode
local algorithm = vals.algorithm
local key       = vals.key
local input     = vals.input

if not key or key == "" then
    ui.error("Error", "Passphrase must not be empty.")
    return
end

if not input or input == "" then
    ui.error("Error", "Input text must not be empty.")
    return
end

local ok, result = pcall(function()
    if mode == "Encrypt" then
        return crypto.encrypt(input, key, algorithm)
    else
        return crypto.decrypt(input, key, algorithm)
    end
end)

if ok and result then
    ui.show(mode .. "ed (" .. algorithm .. ")", result)
    app.clipboard(result)
    ui.append(mode .. " complete (" .. algorithm .. "). Result copied to clipboard.")
else
    ui.error(mode .. " Failed", tostring(result))
end
