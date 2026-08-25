#!/usr/bin/env python3
"""Reproduce the documented W4DJ RMS-squared Energy calibration.

Usage:
    python3 docs/analysis/energy-calibration-reference.py \
        /path/to/full-library-observations.csv

The input observations CSV must expose status and energy columns. Ratings are
loaded from the adjacent data/energy-calibration-ratings.csv file.
"""

from __future__ import annotations

import argparse
import bisect
import csv
import math
from pathlib import Path


TARGET_PROPORTIONS = (0.06, 0.08, 0.09, 0.11, 0.13, 0.14, 0.13, 0.11, 0.09, 0.06)
BASE_GAMMA = 100.0
BASE_LABEL_COUNT = 22
MIN_BIN_PROPORTION = 0.02
MAX_BIN_PROPORTION = 0.28
HUBER_DELTA = 1.5


def load_energies(path: Path) -> list[float]:
    values: list[float] = []
    with path.open(encoding="utf-8-sig", newline="") as handle:
        for row in csv.DictReader(handle):
            if row.get("status") != "completed" or not row.get("energy"):
                continue
            value = float(row["energy"])
            if math.isfinite(value) and value > 0:
                values.append(value)
    if not values:
        raise ValueError("observations CSV has no completed finite positive Energy rows")
    return sorted(values)


def load_ratings(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if len(rows) != 49 or len({row["id"] for row in rows}) != 49:
        raise ValueError("ratings CSV must contain the documented 49 unique IDs")
    if any(not 1 <= int(row["rating"]) <= 10 for row in rows):
        raise ValueError("ratings must be integers from 1 through 10")
    return rows


def huber(residual: float) -> float:
    absolute = abs(residual)
    if absolute <= HUBER_DELTA:
        return 0.5 * absolute * absolute
    return HUBER_DELTA * (absolute - 0.5 * HUBER_DELTA)


def solve(
    energies: list[float],
    ratings: list[dict[str, str]],
    skipped_groups: set[int] | None = None,
) -> tuple[list[float], list[int]]:
    skipped_groups = skipped_groups or set()
    total = len(energies)
    gamma = BASE_GAMMA * len(ratings) / BASE_LABEL_COUNT
    minimum = round(MIN_BIN_PROPORTION * total)
    maximum = round(MAX_BIN_PROPORTION * total)
    labels_by_rank: dict[int, list[int]] = {}

    for row in ratings:
        group = int(row["boundary_group"]) if row["boundary_group"] else 0
        if group in skipped_groups:
            continue
        rank = bisect.bisect_left(energies, float(row["energy"]))
        labels_by_rank.setdefault(rank, []).append(int(row["rating"]))

    prefix = [[0.0] * (total + 1) for _ in range(11)]
    for level in range(1, 11):
        running = 0.0
        for rank in range(total):
            for rating in labels_by_rank.get(rank, ()):
                running += huber(level - rating)
            prefix[level][rank + 1] = running

    infinity = float("inf")
    previous = [infinity] * (total + 1)
    previous[0] = 0.0
    back_pointers: list[list[int]] = []

    for level in range(1, 11):
        current = [infinity] * (total + 1)
        back = [-1] * (total + 1)
        for end in range(level * minimum, min(total, level * maximum) + 1):
            start_low = max((level - 1) * minimum, end - maximum)
            start_high = min((level - 1) * maximum, end - minimum)
            for start in range(start_low, start_high + 1):
                if not math.isfinite(previous[start]):
                    continue
                proportion = (end - start) / total
                distribution_cost = gamma * (
                    (proportion - TARGET_PROPORTIONS[level - 1]) ** 2
                    / TARGET_PROPORTIONS[level - 1]
                )
                candidate = (
                    previous[start]
                    + prefix[level][end]
                    - prefix[level][start]
                    + distribution_cost
                )
                if candidate < current[end]:
                    current[end] = candidate
                    back[end] = start
        previous = current
        back_pointers.append(back)

    if not math.isfinite(previous[total]):
        raise RuntimeError("no valid ten-bin calibration was found")

    bins: list[tuple[int, int]] = []
    end = total
    for level in range(10, 0, -1):
        start = back_pointers[level - 1][end]
        bins.append((start, end))
        end = start
    bins.reverse()

    thresholds = [energies[end - 1] for _, end in bins[:-1]]
    counts = [end - start for start, end in bins]
    return thresholds, counts


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("observations_csv", type=Path)
    parser.add_argument(
        "--ratings",
        type=Path,
        default=Path(__file__).parent / "data" / "energy-calibration-ratings.csv",
    )
    args = parser.parse_args()

    energies = load_energies(args.observations_csv)
    ratings = load_ratings(args.ratings)
    thresholds, counts = solve(energies, ratings)

    print(f"valid_energy_rows={len(energies)} ratings={len(ratings)}")
    print("thresholds=" + ",".join(f"{value:.9f}" for value in thresholds))
    print("counts=" + ",".join(str(value) for value in counts))
    print(
        "percentages="
        + ",".join(f"{100 * value / len(energies):.2f}" for value in counts)
    )

    group_results = [solve(energies, ratings, {group})[0] for group in range(1, 10)]
    shifts: list[float] = []
    for index, baseline in enumerate(thresholds):
        maximum_shift = max(
            abs(result[index] - baseline) / baseline for result in group_results
        )
        shifts.append(100 * maximum_shift)
    print("group_deletion_max_shift_pct=" + ",".join(f"{value:.3f}" for value in shifts))


if __name__ == "__main__":
    main()
