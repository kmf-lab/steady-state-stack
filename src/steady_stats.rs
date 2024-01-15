use num_traits::{FromPrimitive, Zero, One, NumCast};
use std::fmt::Debug;
use std::error::Error;
use num_traits::real::Real;
use std::collections::HashMap;
use std::ops::{AddAssign, Div, Mul};

#[derive(Debug, Clone, Copy)]
struct RunningStats<I, F>
    where
        I: Zero + One + Copy + Debug + NumCast,
        F: FromPrimitive + Zero + Copy + Debug + NumCast + Real,
{
    n: I,
    mean: F,
    m2: F,
}

impl<I, F> RunningStats<I, F>
    where
        I: Zero + One + Copy + Debug + NumCast + PartialOrd,
        F: FromPrimitive + Zero + Copy + Debug + NumCast + Real,
{
    fn new() -> Self {
        RunningStats {
            n: I::zero(),
            mean: F::zero(),
            m2: F::zero(),
        }
    }

    fn update(&mut self, value: F) -> Result<(), Box<dyn Error>> {
        let n_as_f: F = NumCast::from(self.n).ok_or("Conversion error")?;
        let delta: F = value - self.mean;
        self.n = self.n + I::one();
        let temp: F = delta / (n_as_f + F::one());
        self.mean = self.mean + temp;
        let delta2: F = value - self.mean;
        self.m2 = self.m2 + delta * delta2;
        Ok(())
    }

    fn reset(&mut self) {
        self.n = I::zero();
        self.mean = F::zero();
        self.m2 = F::zero();
    }

    fn mean(&self) -> F {
        self.mean
    }

    fn variance(&self) -> Result<F, Box<dyn Error>> {
        if self.n <= (I::one()) {
            Ok(F::zero())
        } else {
            let n_as_f: F = NumCast::from(self.n).ok_or("Conversion error")?;
            Ok(self.m2 / n_as_f)
        }
    }

    fn standard_deviation(&self) -> Result<F, Box<dyn Error>> {
        Ok(self.variance()?.sqrt())
    }
}
//////////////////////////////////////////////////////////////
/////////////////////////////////////////////////////////////


#[derive(Debug, Clone)]
struct Histogram<T, C>
    where
        T: Eq + std::hash::Hash + Ord + Copy,
        C: Zero + One + NumCast + Copy + Ord + AddAssign + Mul<Output = C>,
{
    frequencies: HashMap<T, C>, //TODO: may use Vec instead? probably yes
    total_count: C,
}

impl<T, C> Histogram<T, C>
    where
        T: Eq + std::hash::Hash + Ord + Copy,
        C: Zero + One + NumCast + Copy
           + Ord + AddAssign + Mul<Output = C> + Div<Output = C>,
{
    fn new() -> Self {
        Histogram {
            frequencies: HashMap::new(),
            total_count: C::zero(),
        }
    }

    fn add(&mut self, value: T) {
        *self.frequencies.entry(value).or_insert(C::zero()) += C::one();
        self.total_count += C::one();
    }

    fn percentile(&self, numer: C, denom: C) -> Result<T, Box<dyn Error>> {
        if self.total_count == C::zero() {
            return Err("Histogram is empty".into());
        }
        if denom == C::zero() {
            return Err("Denominator cannot be zero".into());
        }

        let mut sorted_values: Vec<_> = self.frequencies.iter().collect();
        sorted_values.sort_by_key(|&(val, _)| val);

        let mut accumulated_count = C::zero();
        let percentile_rank = (self.total_count * numer) / denom;

        for &(value, count) in sorted_values.iter() {
            accumulated_count += *count;
            if accumulated_count >= percentile_rank {
                return Ok(*value);
            }
        }

        Err("Unable to calculate percentile".into())
    }
}

////////////////////////////////////////////////////
///////////////////////////////////////////////////
#[cfg(test)]
mod tests {
    use super::*;

    /// Test the functionality of the RunningStats struct.
    #[test]
    fn test_std_run() {
        let mut stats = RunningStats::<u32, f64>::new();

        // Example usage
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        for value in data {
            assert!(stats.update(value).is_ok());
        }

        assert_eq!(stats.mean(), 3.0);
        assert_eq!(stats.variance().unwrap(), 2.0);
        assert_eq!(stats.standard_deviation().unwrap(), 1.4142135623730951);

        // Resetting the statistics
        stats.reset();
        assert_eq!(stats.mean(), 0.0);
    }

    /// Test the functionality of the Histogram struct.
    #[test]
    fn test_hist_run() {
        let mut hist = Histogram::<i32, u32>::new();

        // Add data to the histogram
        for value in 1..=10 {
            hist.add(value);
        }

        // Example of calculating the 96th percentile
        assert_eq!(hist.percentile(96, 100).unwrap(), 9);
    }
}
