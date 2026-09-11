def total(items, discount_bps=0):
    return int(sum(quantity * price for quantity, price in items) * (1 - discount_bps / 100))
