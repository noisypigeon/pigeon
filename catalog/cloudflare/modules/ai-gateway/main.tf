# One AI Gateway, authenticated, on stored keys only; see
# .claude/decisions/2026-11-16-security-ai-gateway-claude-api.md.
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "5.28.0"
    }
  }
}

resource "cloudflare_ai_gateway" "this" {
  account_id                 = var.account_id
  id                         = var.name
  authentication             = true
  byok_only                  = true
  store_id                   = var.store_id
  collect_logs               = true
  cache_ttl                  = 0
  cache_invalidate_on_update = false
  rate_limiting_interval     = 60
  rate_limiting_limit        = var.rate_limit_per_minute
  rate_limiting_technique    = "sliding"
  retry_max_attempts         = 1
  log_management             = 100000
  log_management_strategy    = "DELETE_OLDEST"
  logpush                    = false
}
