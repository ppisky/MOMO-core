"""Compatibility import for the superseded MORP 0.2 development corpus.

New builds use MORP 1.0. The old module path remains importable so downstream
tools fail forward to the role-play-only corpus instead of silently recreating
the memory-optimized 0.2 design.
"""

from .roleplay_v1 import make_roleplay_v1_cases


def make_roleplay_v02_cases():
    return make_roleplay_v1_cases()
