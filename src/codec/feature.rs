use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalFeatureReportRequest {
    pub report_id: u8,
    pub size: usize,
}

pub(super) const DS5_PHYSICAL_FEATURE_REPORTS_TO_CACHE: [PhysicalFeatureReportRequest; 2] = [
    PhysicalFeatureReportRequest {
        report_id: 0x05,
        size: 41,
    },
    PhysicalFeatureReportRequest {
        report_id: 0x20,
        size: 64,
    },
];

#[derive(Debug, Default)]
pub struct FeatureReportCache {
    reports: HashMap<u8, Vec<u8>>,
}

impl FeatureReportCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, report_id: u8, data: Vec<u8>) {
        self.reports.insert(report_id, data);
    }

    pub fn get(&self, report_id: u8) -> Option<&[u8]> {
        self.reports.get(&report_id).map(Vec::as_slice)
    }
}

pub(super) fn report_with_id(report_id: u8, data: &[u8]) -> Vec<u8> {
    if data.first() == Some(&report_id) {
        data.to_vec()
    } else {
        let mut full_data = Vec::with_capacity(data.len() + 1);
        full_data.push(report_id);
        full_data.extend_from_slice(data);
        full_data
    }
}
