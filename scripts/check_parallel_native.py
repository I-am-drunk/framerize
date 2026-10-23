"""Explicit entry point. Importing the checker never operates a service."""
from parallel_native_check.runner import main

if __name__ == '__main__':
    raise SystemExit(main())
