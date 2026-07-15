from __future__ import annotations

import random
from collections.abc import Sequence

import numpy as np


def median(values: Sequence[float]) -> float | None:
    return None if not values else float(np.median(np.asarray(values, dtype=float)))


def percentile(values: Sequence[float], quantile: float) -> float | None:
    return (
        None
        if not values
        else float(np.percentile(np.asarray(values, dtype=float), quantile))
    )


def bootstrap_median_ratio(
    numerator: Sequence[float],
    denominator: Sequence[float],
    *,
    samples: int = 10_000,
    seed: int = 23,
) -> tuple[float, float] | None:
    if not numerator or not denominator:
        return None
    rng = random.Random(seed)
    ratios: list[float] = []
    for _ in range(samples):
        left = [numerator[rng.randrange(len(numerator))] for _ in numerator]
        right = [denominator[rng.randrange(len(denominator))] for _ in denominator]
        right_median = float(np.median(right))
        if right_median:
            ratios.append(float(np.median(left)) / right_median)
    if not ratios:
        return None
    return (float(np.percentile(ratios, 2.5)), float(np.percentile(ratios, 97.5)))

