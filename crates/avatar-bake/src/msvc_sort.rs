//! Ports MSVC STL std::sort (introsort: insertion below 32, ninther pivot, heap fallback) used by avatar_export.cpp.
//! Equal keys land where the C++ puts them, which decides atlas chart placement.

const ISORT_MAX: usize = 32;

/// std::sort(v.begin(), v.end(), less).
pub fn sort_by<T: Clone, F: FnMut(&T, &T) -> bool>(v: &mut [T], mut less: F) {
    let n = v.len() as isize;
    sort_unchecked(v, n, &mut less);
}

fn sort_unchecked<T: Clone, F: FnMut(&T, &T) -> bool>(v: &mut [T], ideal: isize, less: &mut F) {
    let mut first = 0usize;
    let mut last = v.len();
    let mut ideal = ideal;
    loop {
        if last - first <= ISORT_MAX {
            insertion_sort(&mut v[first..last], less);
            return;
        }
        if ideal <= 0 {
            make_heap(&mut v[first..last], less);
            sort_heap(&mut v[first..last], less);
            return;
        }
        let (m1, m2) = partition(&mut v[first..last], less);
        let (m1, m2) = (m1 + first, m2 + first);
        ideal = (ideal >> 1) + (ideal >> 2);
        if m1 - first < last - m2 {
            sort_unchecked(&mut v[first..m1], ideal, less);
            first = m2;
        } else {
            sort_unchecked(&mut v[m2..last], ideal, less);
            last = m1;
        }
    }
}

fn insertion_sort<T: Clone, F: FnMut(&T, &T) -> bool>(v: &mut [T], less: &mut F) {
    for mid in 1..v.len() {
        let val = v[mid].clone();
        if less(&val, &v[0]) {
            v[..=mid].rotate_right(1);
        } else {
            let mut hole = mid;
            while less(&val, &v[hole - 1]) {
                v[hole] = v[hole - 1].clone();
                hole -= 1;
            }
            v[hole] = val;
        }
    }
}

fn med3<T, F: FnMut(&T, &T) -> bool>(v: &mut [T], first: usize, mid: usize, last: usize, less: &mut F) {
    if less(&v[mid], &v[first]) {
        v.swap(mid, first);
    }
    if less(&v[last], &v[mid]) {
        v.swap(last, mid);
        if less(&v[mid], &v[first]) {
            v.swap(mid, first);
        }
    }
}

fn guess_median<T, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    mid: usize,
    last: usize,
    less: &mut F,
) {
    let count = last - first;
    if count > 40 {
        let step = (count + 1) >> 3;
        let two_step = step << 1;
        med3(v, first, first + step, first + two_step, less);
        med3(v, mid - step, mid, mid + step, less);
        med3(v, last - two_step, last - step, last, less);
        med3(v, first + step, mid, last - step, less);
    } else {
        med3(v, first, mid, last, less);
    }
}

fn partition<T, F: FnMut(&T, &T) -> bool>(v: &mut [T], less: &mut F) -> (usize, usize) {
    let n = v.len();
    let mid = n >> 1;
    guess_median(v, 0, mid, n - 1, less);
    let mut pfirst = mid;
    let mut plast = pfirst + 1;
    while 0 < pfirst && !less(&v[pfirst - 1], &v[pfirst]) && !less(&v[pfirst], &v[pfirst - 1]) {
        pfirst -= 1;
    }
    while plast < n && !less(&v[plast], &v[pfirst]) && !less(&v[pfirst], &v[plast]) {
        plast += 1;
    }
    let mut gfirst = plast;
    let mut glast = pfirst;
    loop {
        while gfirst < n {
            if less(&v[pfirst], &v[gfirst]) {
            } else if less(&v[gfirst], &v[pfirst]) {
                break;
            } else if plast != gfirst {
                v.swap(plast, gfirst);
                plast += 1;
            } else {
                plast += 1;
            }
            gfirst += 1;
        }
        while 0 < glast {
            let gp = glast - 1;
            if less(&v[gp], &v[pfirst]) {
            } else if less(&v[pfirst], &v[gp]) {
                break;
            } else {
                pfirst -= 1;
                if pfirst != gp {
                    v.swap(pfirst, gp);
                }
            }
            glast -= 1;
        }
        if glast == 0 && gfirst == n {
            return (pfirst, plast);
        }
        if glast == 0 {
            if plast != gfirst {
                v.swap(pfirst, plast);
            }
            plast += 1;
            v.swap(pfirst, gfirst);
            pfirst += 1;
            gfirst += 1;
        } else if gfirst == n {
            glast -= 1;
            pfirst -= 1;
            if glast != pfirst {
                v.swap(glast, pfirst);
            }
            plast -= 1;
            v.swap(pfirst, plast);
        } else {
            glast -= 1;
            v.swap(gfirst, glast);
            gfirst += 1;
        }
    }
}

fn push_heap_by_index<T: Clone, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    mut hole: isize,
    top: isize,
    val: T,
    less: &mut F,
) {
    let mut idx = (hole - 1) >> 1;
    while top < hole && less(&v[idx as usize], &val) {
        v[hole as usize] = v[idx as usize].clone();
        hole = idx;
        idx = (hole - 1) >> 1;
    }
    v[hole as usize] = val;
}

fn pop_heap_hole_by_index<T: Clone, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    mut hole: isize,
    bottom: isize,
    val: T,
    less: &mut F,
) {
    let top = hole;
    let mut idx = hole;
    let max_non_leaf = (bottom - 1) >> 1;
    while idx < max_non_leaf {
        idx = 2 * idx + 2;
        if less(&v[idx as usize], &v[(idx - 1) as usize]) {
            idx -= 1;
        }
        v[hole as usize] = v[idx as usize].clone();
        hole = idx;
    }
    if idx == max_non_leaf && bottom % 2 == 0 {
        v[hole as usize] = v[(bottom - 1) as usize].clone();
        hole = bottom - 1;
    }
    push_heap_by_index(v, hole, top, val, less);
}

fn make_heap<T: Clone, F: FnMut(&T, &T) -> bool>(v: &mut [T], less: &mut F) {
    let bottom = v.len() as isize;
    let mut hole = bottom >> 1;
    while hole > 0 {
        hole -= 1;
        let val = v[hole as usize].clone();
        pop_heap_hole_by_index(v, hole, bottom, val, less);
    }
}

fn sort_heap<T: Clone, F: FnMut(&T, &T) -> bool>(v: &mut [T], less: &mut F) {
    let mut last = v.len();
    while last >= 2 {
        let end = last - 1;
        let val = v[end].clone();
        v[end] = v[0].clone();
        pop_heap_hole_by_index(&mut v[..end], 0, end as isize, val, less);
        last -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_a_sort() {
        let mut seed = 12345u32;
        for n in [0usize, 1, 5, 33, 41, 100, 1000] {
            let mut v: Vec<u32> = (0..n)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (seed >> 16) % 17
                })
                .collect();
            let mut want = v.clone();
            want.sort();
            sort_by(&mut v, |a, b| a < b);
            assert_eq!(v, want);
        }
    }

    #[test]
    fn heap_fallback_sorts() {
        let mut v: Vec<i32> = (0..200).rev().collect();
        make_heap(&mut v, &mut |a: &i32, b: &i32| a < b);
        sort_heap(&mut v, &mut |a: &i32, b: &i32| a < b);
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
    }
}
