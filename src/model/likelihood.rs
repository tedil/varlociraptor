use bio::stats::{bayesian::model::Likelihood, LogProb};

use crate::model::evidence::Observation;
use crate::model::sample::Pileup;
use crate::model::AlleleFreq;

fn prob_sample_alt(observation: &Observation, allele_freq: LogProb) -> LogProb {
    if allele_freq != LogProb::ln_one() {
        // The effective sample probability for the alt allele is the allele frequency times
        // the probability to obtain a feasible fragment (prob_sample_alt).
        allele_freq + observation.prob_sample_alt
    } else {
        // If allele frequency is 1.0, sampling bias does have no effect because all reads
        // should come from the alt allele.
        allele_freq
    }
}

/// Variant calling model, taking purity and allele frequencies into account.
#[derive(Clone, Copy, Debug)]
pub struct ContaminatedSampleLikelihoodModel {
    /// Purity of the case sample.
    purity: LogProb,
    impurity: LogProb,
}

impl Default for ContaminatedSampleLikelihoodModel {
    fn default() -> Self {
        ContaminatedSampleLikelihoodModel::new(1.0)
    }
}

impl ContaminatedSampleLikelihoodModel {
    /// Create new model.
    pub fn new(purity: f64) -> Self {
        assert!(purity > 0.0 && purity <= 1.0);
        let purity = LogProb(purity.ln());
        ContaminatedSampleLikelihoodModel {
            purity: purity,
            impurity: purity.ln_one_minus_exp(),
        }
    }

    fn likelihood_observation(
        &self,
        allele_freq: LogProb,
        allele_freq_contamination: LogProb,
        observation: &Observation,
    ) -> LogProb {
        // Step 1: probability to sample observation: AF * placement induced probability
        let prob_sample_alt_case = prob_sample_alt(observation, allele_freq);
        let prob_sample_alt_control = prob_sample_alt(observation, allele_freq_contamination);

        // Step 2: read comes from control sample and is correctly mapped
        let prob_control = self.impurity
            + (prob_sample_alt_control + observation.prob_alt)
                .ln_add_exp(prob_sample_alt_control.ln_one_minus_exp() + observation.prob_ref);
        assert!(!prob_control.is_nan());

        // Step 3: read comes from case sample and is correctly mapped
        let prob_case = self.purity
            + (prob_sample_alt_case + observation.prob_alt)
                .ln_add_exp(prob_sample_alt_case.ln_one_minus_exp() + observation.prob_ref);
        assert!(!prob_case.is_nan());

        // Step 4: total probability
        // Important note: we need to multiply a probability for a hypothetical missed allele
        // in the mismapping case. Otherwise, it can happen that mismapping dominates subtle
        // differences in the likelihood for alt and ref allele with low probabilities and very
        // low allele frequencies, such that we loose sensitivity for those.
        let total = (observation.prob_mapping + prob_control.ln_add_exp(prob_case))
            .ln_add_exp(observation.prob_mismapping + observation.prob_missed_allele);
        assert!(!total.is_nan());
        total
    }
}

impl Likelihood for ContaminatedSampleLikelihoodModel {
    type Event = (AlleleFreq, AlleleFreq);
    type Data = Pileup;

    fn compute(&self, allelefreqs: &(AlleleFreq, AlleleFreq), pileup: &Pileup) -> LogProb {
        let (allele_freq, allele_freq_contamination) = allelefreqs;
        let ln_af = LogProb(allele_freq.ln());
        let ln_af_contamination = LogProb(allele_freq_contamination.ln());
        // calculate product of per-oservation likelihoods in log space
        let likelihood = pileup.iter().fold(LogProb::ln_one(), |prob, obs| {
            let lh = self.likelihood_observation(ln_af, ln_af_contamination, obs);
            prob + lh
        });

        assert!(!likelihood.is_nan());
        likelihood
    }
}

/// Likelihood model for single sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct SampleLikelihoodModel {}

impl SampleLikelihoodModel {
    /// Create new model.
    pub fn new() -> Self {
        SampleLikelihoodModel {}
    }

    /// Likelihood to observe a read given allele frequency for a single sample.
    fn likelihood_observation(&self, allele_freq: LogProb, observation: &Observation) -> LogProb {
        // Step 1: calculate probability to sample from alt allele
        let prob_sample_alt = prob_sample_alt(observation, allele_freq);

        // Step 2: read comes from case sample and is correctly mapped
        let prob_case = (prob_sample_alt + observation.prob_alt)
            .ln_add_exp(prob_sample_alt.ln_one_minus_exp() + observation.prob_ref);
        assert!(!prob_case.is_nan());

        // Step 3: total probability
        // Important note: we need to multiply a probability for a hypothetical missed allele
        // in the mismapping case. Otherwise, it can happen that mismapping dominates subtle
        // differences in the likelihood for alt and ref allele with low probabilities and very
        // low allele frequencies, such that we loose sensitivity for those.
        let total = (observation.prob_mapping + prob_case)
            .ln_add_exp(observation.prob_mismapping + observation.prob_missed_allele);
        assert!(!total.is_nan());
        total
    }
}

impl Likelihood for SampleLikelihoodModel {
    type Event = AlleleFreq;
    type Data = Pileup;

    /// Likelihood to observe a pileup given allele frequencies for case and control.
    fn compute(&self, allele_freq: &AlleleFreq, pileup: &Pileup) -> LogProb {
        let ln_af = LogProb(allele_freq.ln());
        // calculate product of per-read likelihoods in log space
        let likelihood = pileup.iter().fold(LogProb::ln_one(), |prob, obs| {
            let lh = self.likelihood_observation(ln_af, obs);
            prob + lh
        });

        assert!(!likelihood.is_nan());
        likelihood
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::observation;
    use bio::stats::LogProb;
    use itertools_num::linspace;

    #[test]
    fn test_likelihood_observation_absent_single() {
        let observation = observation(LogProb::ln_one(), LogProb::ln_zero(), LogProb::ln_one());

        let lh = LatentVariableModel::likelihood_observation_single_sample(
            &observation,
            LogProb(AlleleFreq(0.0).ln()),
        );
        assert_relative_eq!(*lh, *LogProb::ln_one());
    }

    #[test]
    fn test_likelihood_observation_absent() {
        let model = LatentVariableModel::new(1.0);
        let observation = observation(LogProb::ln_one(), LogProb::ln_zero(), LogProb::ln_one());

        let lh = model.likelihood_observation_case_control(
            &observation,
            LogProb(AlleleFreq(0.0).ln()),
            LogProb(AlleleFreq(0.0).ln()),
        );
        assert_relative_eq!(*lh, *LogProb::ln_one());
    }

    #[test]
    fn test_likelihood_pileup_absent() {
        let model = LatentVariableModel::new(1.0);
        let mut observations = Vec::new();
        for _ in 0..10 {
            observations.push(observation(
                LogProb::ln_one(),
                LogProb::ln_zero(),
                LogProb::ln_one(),
            ));
        }

        let lh = model.likelihood_pileup(&observations, AlleleFreq(0.0), Some(AlleleFreq(0.0)));
        assert_relative_eq!(*lh, *LogProb::ln_one());
    }

    #[test]
    fn test_likelihood_pileup_absent_single() {
        let model = LatentVariableModel::new(1.0);
        let mut observations = Vec::new();
        for _ in 0..10 {
            observations.push(observation(
                LogProb::ln_one(),
                LogProb::ln_zero(),
                LogProb::ln_one(),
            ));
        }

        let lh = model.likelihood_pileup(&observations, AlleleFreq(0.0), None);
        assert_relative_eq!(*lh, *LogProb::ln_one());
    }

    #[test]
    fn test_likelihood_observation_case_control() {
        let model = LatentVariableModel::new(1.0);
        let observation = observation(LogProb::ln_one(), LogProb::ln_one(), LogProb::ln_zero());

        let lh = model.likelihood_observation_case_control(
            &observation,
            LogProb(AlleleFreq(1.0).ln()),
            LogProb(AlleleFreq(0.0).ln()),
        );
        assert_relative_eq!(*lh, *LogProb::ln_one());

        let lh = model.likelihood_observation_case_control(
            &observation,
            LogProb(AlleleFreq(0.0).ln()),
            LogProb(AlleleFreq(0.0).ln()),
        );
        assert_relative_eq!(*lh, *LogProb::ln_zero());

        let lh = model.likelihood_observation_case_control(
            &observation,
            LogProb(AlleleFreq(0.5).ln()),
            LogProb(AlleleFreq(0.0).ln()),
        );
        assert_relative_eq!(*lh, 0.5f64.ln());

        let lh = model.likelihood_observation_case_control(
            &observation,
            LogProb(AlleleFreq(0.5).ln()),
            LogProb(AlleleFreq(0.5).ln()),
        );
        assert_relative_eq!(*lh, 0.5f64.ln());

        let lh = model.likelihood_observation_case_control(
            &observation,
            LogProb(AlleleFreq(0.1).ln()),
            LogProb(AlleleFreq(0.0).ln()),
        );
        assert_relative_eq!(*lh, 0.1f64.ln());

        // test with 50% purity
        let model = LatentVariableModel::new(0.5);

        let lh = model.likelihood_observation_case_control(
            &observation,
            LogProb(AlleleFreq(0.0).ln()),
            LogProb(AlleleFreq(1.0).ln()),
        );
        assert_relative_eq!(*lh, 0.5f64.ln(), epsilon = 0.0000000001);
    }

    #[test]
    fn test_likelihood_observation_single_sample() {
        let observation = observation(
            // prob_mapping
            LogProb::ln_one(),
            // prob_alt
            LogProb::ln_one(),
            // prob_ref
            LogProb::ln_zero(),
        );

        let lh = LatentVariableModel::likelihood_observation_single_sample(
            &observation,
            LogProb(AlleleFreq(1.0).ln()),
        );
        assert_relative_eq!(*lh, *LogProb::ln_one());

        let lh = LatentVariableModel::likelihood_observation_single_sample(
            &observation,
            LogProb(AlleleFreq(0.0).ln()),
        );
        assert_relative_eq!(*lh, *LogProb::ln_zero());

        let lh = LatentVariableModel::likelihood_observation_single_sample(
            &observation,
            LogProb(AlleleFreq(0.5).ln()),
        );
        assert_relative_eq!(*lh, 0.5f64.ln());

        let lh = LatentVariableModel::likelihood_observation_single_sample(
            &observation,
            LogProb(AlleleFreq(0.1).ln()),
        );
        assert_relative_eq!(*lh, 0.1f64.ln());
    }

    #[test]
    fn test_likelihood_pileup() {
        let model = LatentVariableModel::new(1.0);
        let mut observations = Vec::new();
        for _ in 0..5 {
            observations.push(observation(
                LogProb::ln_one(),
                LogProb::ln_one(),
                LogProb::ln_zero(),
            ));
        }
        for _ in 0..5 {
            observations.push(observation(
                LogProb::ln_one(),
                LogProb::ln_zero(),
                LogProb::ln_one(),
            ));
        }
        let lh = model.likelihood_pileup(&observations, AlleleFreq(0.5), Some(AlleleFreq(0.0)));
        for af in linspace(0.0, 1.0, 10) {
            if af != 0.5 {
                let l =
                    model.likelihood_pileup(&observations, AlleleFreq(af), Some(AlleleFreq(0.0)));
                assert!(lh > l);
            }
        }
    }
}
