"""Opt-in hook for an explicitly packaged resident diagnostic host."""


def install(operation_type, record, describe, emit):
    """Observe successful existing mints; return an ownership-aware restore hook."""
    original = operation_type.charged_mint

    async def observed(operation):
        token = await original(operation)
        try:
            phase = 'baseline' if operation.phase == 'baseline_pending' else 'native'
            record(token, operation.identity['lease'], phase, describe, emit)
        except Exception:
            pass
        return token

    operation_type.charged_mint = observed

    def restore():
        if operation_type.charged_mint is not observed:
            return False
        operation_type.charged_mint = original
        return True

    return restore
