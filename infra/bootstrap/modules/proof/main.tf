# Proof unit; see .claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md.
variable "message" {
  type = string
}

resource "terraform_data" "this" {
  input = var.message
}
