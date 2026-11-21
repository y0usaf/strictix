local M = {}

local function sum(xs)
  local total = 0
  for _, x in ipairs(xs) do
    total = total + x
  end
  return total
end

M.sum = sum

return M
