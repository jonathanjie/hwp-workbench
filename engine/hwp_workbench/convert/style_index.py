"""Value-keyed indexes preserve first-use order without repeated list scans."""


def immutable_value(value):
    if isinstance(value, dict):
        return tuple((key, immutable_value(item)) for key, item in sorted(value.items()))
    if isinstance(value, (tuple, list)):
        return tuple(immutable_value(item) for item in value)
    return value


def indexed_value(items, indices, value):
    key = immutable_value(value)
    if key not in indices:
        indices[key] = len(items)
        items.append(value)
    return indices[key]
