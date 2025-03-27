pub fn is_true(v:&bool) -> bool {
    return *v;
}

pub fn is_false(v:&bool) -> bool {
    return !*v;
}

pub fn bool_default_true() -> bool {
    return true;
}

pub fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    t == &T::default()
}