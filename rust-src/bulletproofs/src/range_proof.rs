use curve_arithmetic::Curve;
use ff::{Field, PrimeField};
use merlin::Transcript;
use crate::transcript::TranscriptProtocol;
use group::{CurveAffine, CurveProjective, EncodedPoint};
use pairing::{
    bls12_381::{
        Bls12, Fq, Fr, FrRepr, G1Affine, G1Compressed, G1Prepared, G2Affine, G2Compressed,
        G2Prepared, G1, G2,
    },
    Engine, PairingCurveAffine,
};
use crate::inner_product_proof::*;
use vector_pedersen_scheme::*;
use curve_arithmetic::Value;
use curve_arithmetic::multiscalar_multiplication;
use curve_arithmetic::multiexp_worker_given_table;
use curve_arithmetic::multiexp_table;
use rand::*;

#[derive(Clone)]
pub struct RangeProof<C:Curve> {
    A: C,
    S: C, 
    T_1: C,
    T_2: C,
    tx: C::Scalar, 
    tx_tilde: C::Scalar,
    e_tilde: C::Scalar,  
    ip_proof: InnerProductProof<C>
}

fn bool_to_int(b: bool) -> u8 {
    match b {
        true => 1,
        false => 0,
    }
}

fn ith_bit(v: u64, i: u8) -> u8{
    bool_to_int(v & (1 << i)!= 0)
}

fn ith_bit_bool(v: u64, i: u8) -> bool{
    v & (1 << i)!= 0
}

fn integer_to_bit_vector_over_prime_field<F:PrimeField>(v: u64, n: u8) -> Vec<F>{
    let mut bv = Vec::with_capacity(n as usize);
    for i in 0..n {
        if ith_bit_bool(v, i) {
            bv.push(F::one());
        }
        else {
            bv.push(F::zero());
        }
    }
    bv
}

fn mul_vectors<F:Field>(a: &[F], b: &[F]) -> Vec<F> {
    if a.len() != b.len(){
        panic!("a and b should have the same length");
    }
    
    let mut c = Vec::with_capacity(a.len());
    for (&x,&y) in a.iter().zip(b.iter()) {
        let mut xy = x;
        xy.mul_assign(&y);
        c.push(xy);
    }
    c
}

fn a_L_a_R<F:Field>(v: u64, n: u8) -> (Vec<F>, Vec<F>){
    let mut a_L = Vec::with_capacity(n as usize);
    let mut a_R = Vec::with_capacity(n as usize);
    for i in 0..n {
        let mut bit = F::zero();
        if ith_bit_bool(v, i) {
            bit = F::one();
        }
        a_L.push(bit);
        bit.sub_assign(&F::one());
        a_R.push(bit);
    }
    (a_L, a_R)
}

fn two_n_vec<F: Field>(n: u8) -> Vec<F>{
    let mut two_n = Vec::with_capacity(n as usize);
    let mut two_i = F::one(); 
    for _ in 0..n {
        two_n.push(two_i);
        two_i.double();
    }
    two_n
}

fn minus_vec<F:Field>(a: &[F], b: &[F]) -> Vec<F>{
    if a.len() != b.len(){
        panic!("a and b should have the same length");
    }
    let mut c = Vec::with_capacity(a.len());
    for i in 0..a.len(){
        let mut c_i = a[i];
        c_i.sub_assign(&b[i]);
        c.push(c_i);
    }
    c
}


#[allow(non_snake_case)]
fn prove<C:Curve, T: Rng>(n: u8, m: u8, v_vec: Vec<u64>, G: Vec<C>, H: Vec<C>, B: C, B_tilde: C, csprng: &mut T, 
    transcript: &mut Transcript) -> (Vec<Commitment<C>>, RangeProof<C>){
    let nm = G.len();
    let mut a_L : Vec<C::Scalar> = Vec::with_capacity(n as usize);
    let mut a_R : Vec<C::Scalar> = Vec::with_capacity(n as usize);
    let mut V_vec : Vec<Commitment<C>> = Vec::with_capacity(m as usize);
    // let mut A_vec : Vec<Commitment<C>> = Vec::with_capacity(m as usize);
    // let mut S_vec : Vec<Commitment<C>> = Vec::with_capacity(m as usize);
    let mut s_L = Vec::with_capacity(n as usize);
    let mut s_R = Vec::with_capacity(n as usize);
    // let mut A = C::zero_point();
    // let mut S = C::zero_point();
    for _ in 0..nm {
        s_L.push(C::generate_scalar(csprng));
        s_R.push(C::generate_scalar(csprng));
    }
    // let mut j = 0;
    let v_keys = CommitmentKey(vec![B], B_tilde);
    let mut v_tilde_vec : Vec<C::Scalar> = Vec::with_capacity(m as usize);
    let mut a_tilde_vec : Vec<C::Scalar> = Vec::with_capacity(m as usize);
    let mut s_tilde_vec : Vec<C::Scalar> = Vec::with_capacity(m as usize);
    // let v_copy = v_vec.clone(); // DEBUG 
    for v in v_vec {
        let (a_L_j, a_R_j) = a_L_a_R(v, n);
        a_L.extend(&a_L_j);
        a_R.extend(&a_R_j);
        // let mut a_j = a_L_j;
        // a_j.extend(&a_R_j);
        // let a_j : Vec<Value<C>> = a_j.iter().map(|&x| Value::new(x)).collect();
        // let n = n as usize;
        // let s_L_j = &s_L[j*n..(j+1)*n];
        // let s_R_j = &s_R[j*n..(j+1)*n];
        // let s_j : Vec<Value<C>> = s_L_j.iter().chain(s_R_j).map(|&x| Value::new(x)).collect();
        // let G_j = &G[j*n..(j+1)*n];
        // let H_j = &H[j*n..(j+1)*n];
        // let G_jH_j : Vec<_> = G_j.iter().chain(H_j).map(|&x| x).collect();
        let v_j_tilde = Randomness::<C>::generate(csprng);
        let a_j_tilde = Randomness::<C>::generate(csprng);
        let s_j_tilde = Randomness::<C>::generate(csprng);
        v_tilde_vec.push(*v_j_tilde);
        a_tilde_vec.push(*a_j_tilde);
        s_tilde_vec.push(*s_j_tilde);

        let v_scalar = C::scalar_from_u64(v);
        let v_value = Value::new(v_scalar);
        let V_j = v_keys.hide(&[v_value], &v_j_tilde);
        transcript.append_point(b"Vj", &V_j.0);
        // println!("Prover's V_{:?} = {:?}", j, V_j);
        // let A_keys = CommitmentKey(G_jH_j, B_tilde);
        // let A_j = A_keys.hide(&a_j, &a_j_tilde);
        // let S_j = A_keys.hide(&s_j, &s_j_tilde);
        V_vec.push(V_j);
        // A_vec.push(A_j);
        // S_vec.push(S_j);
        // A = A.plus_point(&A_j.0);
        // S = S.plus_point(&S_j.0);
        // j+=1;
    }
    let mut a_tilde_sum = C::Scalar::zero();
    let mut s_tilde_sum = C::Scalar::zero();
    for i in 0..a_tilde_vec.len(){
        a_tilde_sum.add_assign(&a_tilde_vec[i]);
        s_tilde_sum.add_assign(&s_tilde_vec[i]);
    }
    //Probably faster:
    // let A = multiscalar_multiplication(&a_L, &G).plus_point(&multiscalar_multiplication(&a_R, &H)).plus_point(&B_tilde.mul_by_scalar(&a_tilde_sum));
    // let S = multiscalar_multiplication(&s_L, &G).plus_point(&multiscalar_multiplication(&s_R, &H)).plus_point(&B_tilde.mul_by_scalar(&s_tilde_sum));
    let mut A_scalars : Vec<C::Scalar> = a_L.iter().chain(a_R.iter()).map(|&x| x).collect();
    A_scalars.push(a_tilde_sum);
    let mut S_scalars : Vec<C::Scalar> = s_L.iter().chain(s_R.iter()).map(|&x| x).collect();
    S_scalars.push(s_tilde_sum);
    let mut GH_B_tilde : Vec<C> = G.iter().chain(H.iter()).map(|&x| x).collect();
    GH_B_tilde.push(B_tilde);
    let window_size = 4;
    let table = multiexp_table(&GH_B_tilde, window_size);
    let A = multiexp_worker_given_table(&A_scalars, &table[..], window_size);
    let S = multiexp_worker_given_table(&S_scalars, &table[..], window_size);
    // println!("AA = A ? {}", AA == A);
    // println!("SS = S ? {}", SS == S);

    // let mut A = A_vec[0].0;
    // for i in 1..(m as usize) {
    //     A = A.plus_point(&A_vec[i].0);
    // }

    // let mut S = S_vec[0].0;
    // for i in 1..(m as usize) {
    //     S = S.plus_point(&S_vec[i].0);
    // }

    
    transcript.append_point(b"A", &A);
    transcript.append_point(b"S", &S);
    let y : C::Scalar = transcript.challenge_scalar::<C>(b"y");
    let z : C::Scalar = transcript.challenge_scalar::<C>(b"z");
    
    let mut l_0 = Vec::with_capacity(nm);
    let mut l_1 = Vec::with_capacity(nm);
    let mut r_0 = Vec::with_capacity(nm);
    let mut r_1 = Vec::with_capacity(nm);
    for i in 0..a_L.len() {
        let mut l_0_i = a_L[i];
        l_0_i.sub_assign(&z);
        l_0.push(l_0_i);
        l_1.push(s_L[i]);
    }

    let mut y_nm : Vec<C::Scalar> = Vec::with_capacity(nm);
    let mut y_i = C::Scalar::one();
    for _ in 0..nm {
        y_nm.push(y_i);
        y_i.mul_assign(&y);
    }

    let mut two_n : Vec<C::Scalar> = Vec::with_capacity((n) as usize);
    let mut two_i = C::Scalar::one();
    for _ in 0..(n) as usize {
        two_n.push(two_i);
        two_i.double();
    }

    let mut z_m : Vec<C::Scalar> = Vec::with_capacity((m) as usize);
    let mut z_j = C::Scalar::one();
    for _ in 0..(m) as usize {
        z_m.push(z_j);
        z_j.mul_assign(&z);
    }

    //r_0 and r_1
    for i in 0..a_R.len() {
        let mut r_0_i = a_R[i];
        r_0_i.add_assign(&z);
        r_0_i.mul_assign(&y_nm[i]);
        let j = i/(n as usize);
        let mut z_jz_2_2_n = z_m[j];
        let two_i = two_n[i%(n as usize)];
        z_jz_2_2_n.mul_assign(&z_m[2]);
        z_jz_2_2_n.mul_assign(&two_i);
        r_0_i.add_assign(&z_jz_2_2_n);
        r_0.push(r_0_i);

        let mut r_1_i = y_nm[i];
        r_1_i.mul_assign(&s_R[i]);
        r_1.push(r_1_i);
    }

    let mut t_0 = Vec::with_capacity(m as usize);
    let mut t_1 = Vec::with_capacity(m as usize);
    let mut t_2 = Vec::with_capacity(m as usize);
    let mut t_1_tilde = Vec::with_capacity(m as usize);
    let mut t_2_tilde = Vec::with_capacity(m as usize);
    // let mut T_1 = C::zero_point();
    // let mut T_2 = C::zero_point();
    
    for j in 0..m as usize {
        let n = n as usize;
        let t_0_j = inner_product(&l_0[j*n..(j+1)*n], &r_0[j*n..(j+1)*n]);
        let t_2_j = inner_product(&l_1[j*n..(j+1)*n], &r_1[j*n..(j+1)*n]);

        let mut t_1_j : C::Scalar = C::Scalar::zero();
        for i in 0..n as usize {
            let mut l_0_j_l_1_j = l_0[j*n+i]; 
            l_0_j_l_1_j.add_assign(&l_1[j*n+i]);
            let mut r_0_j_r_1_j = r_0[j*n+i]; 
            r_0_j_r_1_j.add_assign(&r_1[j*n+i]);
            let mut prod = l_0_j_l_1_j;
            prod.mul_assign(&r_0_j_r_1_j);
            t_1_j.add_assign(&prod);
        }
        t_1_j.sub_assign(&t_0_j);
        t_1_j.sub_assign(&t_2_j);

        t_0.push(t_0_j);
        t_1.push(t_1_j);
        t_2.push(t_2_j);

        let t_1_j_tilde = Randomness::<C>::generate(csprng);
        let t_2_j_tilde = Randomness::<C>::generate(csprng);
        // let t_1_j_value : Value<C> = Value::new(t_1_j);
        // let t_2_j_value : Value<C> = Value::new(t_2_j);
        // let T_1_j = v_keys.hide(&[t_1_j_value], &t_1_j_tilde); // This line and the below: 107 ms vs 125 ms
        // let T_2_j = v_keys.hide(&[t_2_j_value], &t_2_j_tilde);
        t_1_tilde.push(t_1_j_tilde);
        t_2_tilde.push(t_2_j_tilde);

        // T_1 = T_1.plus_point(&T_1_j.0); // We could use msm here using the scalars from above
        // T_2 = T_2.plus_point(&T_2_j.0);
    }

    let mut t_1_sum = C::Scalar::zero();
    let mut t_1_tilde_sum = C::Scalar::zero();
    let mut t_2_sum = C::Scalar::zero();
    let mut t_2_tilde_sum = C::Scalar::zero();
    for i in 0..t_1.len(){
        t_1_sum.add_assign(&t_1[i]);
        t_1_tilde_sum.add_assign(&t_1_tilde[i]);
        t_2_sum.add_assign(&t_2[i]);
        t_2_tilde_sum.add_assign(&t_2_tilde[i]);
    }
    let T_1 = B.mul_by_scalar(&t_1_sum).plus_point(&B_tilde.mul_by_scalar(&t_1_tilde_sum));
    let T_2 = B.mul_by_scalar(&t_2_sum).plus_point(&B_tilde.mul_by_scalar(&t_2_tilde_sum));
    // println!("T1 == TT1 ? {}", T_1 == TT1);
    // println!("T2 == TT2 ? {}", T_2 == TT2);

    transcript.append_point(b"T1", &T_1);
    transcript.append_point(b"T2", &T_2);
    let x : C::Scalar = transcript.challenge_scalar::<C>(b"x");
    // println!("prover's x = {:?}", x);
    let mut x2 = x;
    x2.mul_assign(&x);
    let mut l : Vec<C::Scalar> = Vec::with_capacity(nm);
    let mut r : Vec<C::Scalar> = Vec::with_capacity(nm);

    for i in 0..nm {
        let mut l_i = l_1[i];
        l_i.mul_assign(&x);
        l_i.add_assign(&l_0[i]);
        let mut r_i = r_1[i];
        r_i.mul_assign(&x);
        r_i.add_assign(&r_0[i]);
        l.push(l_i);
        r.push(r_i);
    }

    let mut tx : C::Scalar = C::Scalar::zero();
    let mut tx_tilde : C::Scalar = C::Scalar::zero();
    let mut e_tilde : C::Scalar = C::Scalar::zero();
    for j in 0..m as usize { // Around 1 ms
        //tx:
        let mut t1jx = t_1[j];
        t1jx.mul_assign(&x);
        let mut t2jx2 = t_2[j];
        t2jx2.mul_assign(&x2);
        let mut tjx = t_0[j];
        tjx.add_assign(&t1jx);
        tjx.add_assign(&t2jx2);
        tx.add_assign(&tjx);

        //tx tilde:
        let mut z2vj_tilde = z_m[2];
        z2vj_tilde.mul_assign(&z_m[j]); // This line is MISSING in the Bulletproof documentation
        z2vj_tilde.mul_assign(&v_tilde_vec[j]);
        let mut xt1j_tilde = x;
        xt1j_tilde.mul_assign(&t_1_tilde[j]);
        let mut x2t2j_tilde = x2;
        x2t2j_tilde.mul_assign(&t_2_tilde[j]);
        let mut txj_tilde = z2vj_tilde;
        txj_tilde.add_assign(&xt1j_tilde);
        txj_tilde.add_assign(&x2t2j_tilde);
        tx_tilde.add_assign(&txj_tilde);

        //e tilde:
        let mut ej_tilde = x;
        ej_tilde.mul_assign(&s_tilde_vec[j]);
        ej_tilde.add_assign(&a_tilde_vec[j]);
        e_tilde.add_assign(&ej_tilde);
    }

   
    
    transcript.append_scalar::<C>(b"tx", &tx);
    transcript.append_scalar::<C>(b"tx_tilde", &tx_tilde);
    transcript.append_scalar::<C>(b"e_tilde", &e_tilde);
    let w : C::Scalar = transcript.challenge_scalar::<C>(b"w");
    let Q = B.mul_by_scalar(&w);
    // let mut H_prime : Vec<C> = Vec::with_capacity(nm);
    let mut H_prime_scalars : Vec<C::Scalar> = Vec::with_capacity(nm);
    let y_inv = y.inverse().unwrap();
    let mut y_inv_i = C::Scalar::one();
    for i in 0..nm {
        // H_prime.push(H[i].mul_by_scalar(&y_inv_i)); // 245 ms vs 126 ms or 625 ms vs 510
        H_prime_scalars.push(y_inv_i);
        y_inv_i.mul_assign(&y_inv);
    }

    // let P_prime = multiscalar_multiplication(&l, &G).plus_point(&multiscalar_multiplication(&r, &H_prime)).plus_point(&Q.mul_by_scalar(&inner_product(&l, &r)));
    // let P_prime = C::zero_point();
    // println!("Prover's P' = 0? {:?}", P_prime.is_zero_point());
    let ip_proof = prove_inner_product_with_scalars(transcript, G, H, &H_prime_scalars, &Q, l, r);
    // let k = nm.next_power_of_two().trailing_zeros() as usize; //This line is also used in Bulletproofs's implementation
    // let ip_proof = InnerProductProof{L: vec![C::zero_point(); k], R: vec![C::zero_point(); k], a: C::Scalar::zero(), b:C::Scalar::zero()};

    (V_vec, RangeProof{A, S, T_1, T_2, tx, tx_tilde, e_tilde, ip_proof})
}

fn cheat_prove<C:Curve, T: Rng>(n: u8, m: u8, v_vec: Vec<u64>, G: Vec<C>, H: Vec<C>, B: C, B_tilde: C, csprng: &mut T, 
    transcript: &mut Transcript) -> (Vec<Commitment<C>>, RangeProof<C>){
    let nm = (n as usize)*(m as usize);
    let v_copy = v_vec.clone();
    let mut V_vec : Vec<Commitment<C>> = Vec::with_capacity(m as usize);
    let mut v_tilde_vec : Vec<C::Scalar> = Vec::with_capacity(m as usize);
    let v_keys = CommitmentKey(vec![B], B_tilde);
    for v in v_vec {
        
        let v_scalar = C::scalar_from_u64(v);
        let v_value = Value::new(v_scalar);
        let v_j_tilde = Randomness::<C>::generate(csprng);
        v_tilde_vec.push(*v_j_tilde);
        let V_j = v_keys.hide(&[v_value], &v_j_tilde);
        transcript.append_point(b"Vj", &V_j.0);
        V_vec.push(V_j);
        // A_vec.push(V_j);
        // S_vec.push(V_j);
    }
    let A = C::zero_point();
    let S = C::zero_point();
    transcript.append_point(b"A", &A);
    transcript.append_point(b"S", &S);
    let y : C::Scalar = transcript.challenge_scalar::<C>(b"y");
    let z : C::Scalar = transcript.challenge_scalar::<C>(b"z");
    let mut z_m : Vec<C::Scalar> = Vec::with_capacity((m) as usize);
    let mut z_j = C::Scalar::one();
    for _ in 0..(m) as usize {
        z_m.push(z_j);
        z_j.mul_assign(&z);
    }
    let T_1 = C::zero_point();
    let T_2 = C::zero_point();

    let mut tx : C::Scalar = C::Scalar::zero();
    let mut tx_tilde : C::Scalar = C::Scalar::zero();
    let mut e_tilde : C::Scalar = C::Scalar::zero();
    transcript.append_point(b"T1", &T_1);
    transcript.append_point(b"T2", &T_2);
    let x : C::Scalar = transcript.challenge_scalar::<C>(b"x");
    println!("Cheating prover's x = {}", x);
    for j in 0..m as usize {
        //tx:
        let mut z2vj = z_m[2];
        z2vj.mul_assign(&z_m[j]); // This line is MISSING in the Bulletproof documentation
        let v_value = C::scalar_from_u64(v_copy[j]);
        z2vj.mul_assign(&v_value);
        let mut tjx = z2vj;
        tx.add_assign(&tjx);

        //tx tilde:
        let mut z2vj_tilde = z_m[2];
        z2vj_tilde.mul_assign(&z_m[j]); // This line is MISSING in the Bulletproof documentation
        z2vj_tilde.mul_assign(&v_tilde_vec[j]);
        let mut txj_tilde = z2vj_tilde;
        tx_tilde.add_assign(&txj_tilde);
    }
    //delta:
    let mut ip_1_y_nm = C::Scalar::zero();
    let mut yi = C::Scalar::one();
    for _ in 0..G.len() {
        ip_1_y_nm.add_assign(&yi);
        yi.mul_assign(&y);
    }
    let mut ip_1_2_n = C::Scalar::zero();
    let mut two_i = C::Scalar::one();
    for _ in 0..n as usize {
        ip_1_2_n.add_assign(&two_i);
        two_i.double();
    }
    let mut sum = C::Scalar::zero();
    let mut zj3 = z_m[3];
    for _ in 0..m {
        sum.add_assign(&zj3);
        zj3.mul_assign(&z);
    } 
    sum.mul_assign(&ip_1_2_n);
    let mut delta_yz = z;
    delta_yz.sub_assign(&z_m[2]);
    delta_yz.mul_assign(&ip_1_y_nm);
    delta_yz.sub_assign(&sum);
    tx.add_assign(&delta_yz);

    let ip_proof = prove_inner_product(transcript, G, H, &C::zero_point(), vec![C::Scalar::zero(); nm], vec![C::Scalar::zero(); nm]);
    (V_vec, RangeProof{A, S, T_1, T_2, tx, tx_tilde, e_tilde, ip_proof})
}

fn verify<C: Curve>(transcript: &mut Transcript, n: u8, commitments: Vec<Commitment<C>>, proof: RangeProof<C>, G: Vec<C>, H: Vec<C>, B: C, B_tilde: C) -> bool{
    let m = commitments.len();
    for V in commitments.clone() {
        transcript.append_point(b"Vj", &V.0);
    }
    let A = proof.A;
    let S = proof.S;
    let T_1 = proof.T_1;
    let T_2 = proof.T_2;
    let tx = proof.tx;
    let tx_tilde = proof.tx_tilde;
    let e_tilde = proof.e_tilde;
    transcript.append_point(b"A", &A);
    transcript.append_point(b"S", &S);
    let y : C::Scalar = transcript.challenge_scalar::<C>(b"y");
    let z : C::Scalar = transcript.challenge_scalar::<C>(b"z");
    let mut z2 = z;
    z2.mul_assign(&z);
    let mut z3 = z2;
    z3.mul_assign(&z);
    transcript.append_point(b"T1", &T_1);
    transcript.append_point(b"T2", &T_2);
    let x : C::Scalar = transcript.challenge_scalar::<C>(b"x");
    let mut x2 = x;
    x2.mul_assign(&x);
    // println!("verifier's x = {:?}", x);
    transcript.append_scalar::<C>(b"tx", &tx);
    transcript.append_scalar::<C>(b"tx_tilde", &tx_tilde);
    transcript.append_scalar::<C>(b"e_tilde", &e_tilde);
    let w : C::Scalar = transcript.challenge_scalar::<C>(b"w");
    //Calculate delta(x,y):
    let mut ip_1_y_nm = C::Scalar::zero();
    let mut yi = C::Scalar::one();
    for _ in 0..G.len() {
        ip_1_y_nm.add_assign(&yi);
        yi.mul_assign(&y);
    }
    let mut ip_1_2_n = C::Scalar::zero();
    let mut two_i = C::Scalar::one();
    for _ in 0..n as usize {
        ip_1_2_n.add_assign(&two_i);
        two_i.double();
    }
    let mut sum = C::Scalar::zero();
    let mut zj3 = z3;
    for _ in 0..m {
        sum.add_assign(&zj3);
        zj3.mul_assign(&z);
    } 
    sum.mul_assign(&ip_1_2_n);
    let mut delta_yz = z;
    delta_yz.sub_assign(&z2);
    delta_yz.mul_assign(&ip_1_y_nm);
    delta_yz.sub_assign(&sum);

    //LHS of check equation 1:
    let LHS = B.mul_by_scalar(&tx).plus_point(&B_tilde.mul_by_scalar(&tx_tilde));
    let mut RHS = C::zero_point();
    let mut zj2 = z2;
    for j in 0..m {
        let Vj = commitments[j].0;
        // println!("V_{:?} = {:?}", j, Vj);
        RHS = RHS.plus_point(&Vj.mul_by_scalar(&zj2));
        zj2.mul_assign(&z);
    }
    RHS = RHS.plus_point(&B.mul_by_scalar(&delta_yz)).plus_point(&T_1.mul_by_scalar(&x)).plus_point(&T_2.mul_by_scalar(&x2));

    println!("--------------- VERIFICATION ----------------");
    // println!("LHS = {:?}", LHS);
    // println!("RHS = {:?}", RHS);
    // println!("Are they equal? {:?}", LHS == RHS);
    let first = LHS.minus_point(&RHS).is_zero_point();
    println!("First check = {:?}", first);
    
    let ip_proof = proof.ip_proof;
    let mut z_2_nm : Vec<C::Scalar> = Vec::with_capacity(G.len());
    let mut y_i = C::Scalar::one();
    let mut z_2_m : Vec<C::Scalar> = Vec::with_capacity(m);
    let mut z_j = z;
    z_j.mul_assign(&z);
    for j in 0..m {
        z_2_m.push(z_j);
        z_j.mul_assign(&z);
    }
    let y_inv = y.inverse().unwrap();
    let two_n : Vec<C::Scalar> = two_n_vec(n);
    for i in 0..G.len(){
        let j = i/ n as usize;
        let mut prod = two_n[i%n as usize];
        prod.mul_assign(&z_2_m[j]);
        prod.mul_assign(&y_i);
        y_i.mul_assign(&y_inv);
        z_2_nm.push(prod);
    }
    let one = C::Scalar::one();
    let ip1z = multiscalar_multiplication(&vec![one; G.len()], &G).mul_by_scalar(&z);
    let ip2z = multiscalar_multiplication(&vec![one; H.len()], &H).mul_by_scalar(&z);
    // println!("len = {}", H.len());
    let ip3 = multiscalar_multiplication(&z_2_nm, &H); //Expensive!
    let P = A.plus_point(&S.mul_by_scalar(&x)).minus_point(&B_tilde.mul_by_scalar(&e_tilde)).minus_point(&ip1z).plus_point(&ip2z).plus_point(&ip3);
    let mut txw = tx;
    txw.mul_assign(&w);
    let Q = B.mul_by_scalar(&w);
    let P_prime = P.plus_point(&Q.mul_by_scalar(&tx));
    // let (u_sq, u_inv_sq, s) = verify_scalars(transcript, n as usize, ip_proof);
    let mut H_prime : Vec<C> = Vec::with_capacity(m*n as usize);
    let y_inv = y.inverse().unwrap();
    let mut y_inv_i = C::Scalar::one();
    for i in 0..m*n as usize {
        H_prime.push(H[i].mul_by_scalar(&y_inv_i));  //Expensive!
        y_inv_i.mul_assign(&y_inv);
    }

    // println!("Verifier's P' = {:?}", P_prime);
    let second : bool = verify_inner_product(transcript, G, H_prime, P_prime, Q, &ip_proof); // Very expensive
    println!("Second check = {:?}", second);
    first && second
}

fn verify_efficient<C: Curve>(transcript: &mut Transcript, n: u8, commitments: Vec<Commitment<C>>, proof: RangeProof<C>, G: Vec<C>, H: Vec<C>, B: C, B_tilde: C) -> bool{
    let m = commitments.len();
    for V in commitments.clone() {
        transcript.append_point(b"Vj", &V.0);
    }
    let A = proof.A;
    let S = proof.S;
    let T_1 = proof.T_1;
    let T_2 = proof.T_2;
    let tx = proof.tx;
    let tx_tilde = proof.tx_tilde;
    let e_tilde = proof.e_tilde;
    transcript.append_point(b"A", &A);
    transcript.append_point(b"S", &S);
    let y : C::Scalar = transcript.challenge_scalar::<C>(b"y");
    let z : C::Scalar = transcript.challenge_scalar::<C>(b"z");
    let mut z2 = z;
    z2.mul_assign(&z);
    let mut z3 = z2;
    z3.mul_assign(&z);
    transcript.append_point(b"T1", &T_1);
    transcript.append_point(b"T2", &T_2);
    let x : C::Scalar = transcript.challenge_scalar::<C>(b"x");
    let mut x2 = x;
    x2.mul_assign(&x);
    // println!("verifier's x = {:?}", x);
    transcript.append_scalar::<C>(b"tx", &tx);
    transcript.append_scalar::<C>(b"tx_tilde", &tx_tilde);
    transcript.append_scalar::<C>(b"e_tilde", &e_tilde);
    let w : C::Scalar = transcript.challenge_scalar::<C>(b"w");
    //Calculate delta(x,y):
    let mut ip_1_y_nm = C::Scalar::zero();
    let mut yi = C::Scalar::one();
    for _ in 0..G.len() {
        ip_1_y_nm.add_assign(&yi);
        yi.mul_assign(&y);
    }
    let mut ip_1_2_n = C::Scalar::zero();
    let mut two_i = C::Scalar::one();
    for _ in 0..n as usize {
        ip_1_2_n.add_assign(&two_i);
        two_i.double();
    }
    let mut sum = C::Scalar::zero();
    let mut zj3 = z3;
    for _ in 0..m {
        sum.add_assign(&zj3);
        zj3.mul_assign(&z);
    } 
    sum.mul_assign(&ip_1_2_n);
    let mut delta_yz = z;
    delta_yz.sub_assign(&z2);
    delta_yz.mul_assign(&ip_1_y_nm);
    delta_yz.sub_assign(&sum);

    //LHS of check equation 1:
    let LHS = B.mul_by_scalar(&tx).plus_point(&B_tilde.mul_by_scalar(&tx_tilde));
    let mut RHS = C::zero_point();
    let mut zj2 = z2;
    for j in 0..m {
        let Vj = commitments[j].0;
        // println!("V_{:?} = {:?}", j, Vj);
        RHS = RHS.plus_point(&Vj.mul_by_scalar(&zj2));
        zj2.mul_assign(&z);
    }
    RHS = RHS.plus_point(&B.mul_by_scalar(&delta_yz)).plus_point(&T_1.mul_by_scalar(&x)).plus_point(&T_2.mul_by_scalar(&x2));

    println!("--------------- VERIFICATION ----------------");
    let first = LHS.minus_point(&RHS).is_zero_point();
    println!("First check = {:?}", first);
    
    let ip_proof = proof.ip_proof;
    let mut H_scalars : Vec<C::Scalar> = Vec::with_capacity(G.len());
    let mut y_i = C::Scalar::one();
    let mut z_2_m : Vec<C::Scalar> = Vec::with_capacity(m);
    let mut z_j = z;
    z_j.mul_assign(&z);
    for j in 0..m {
        z_2_m.push(z_j);
        z_j.mul_assign(&z);
    }
    let (u_sq, u_inv_sq, s) = verify_scalars(transcript, G.len(), &ip_proof.clone());
    let a = ip_proof.a;
    let b = ip_proof.b;
    let L = ip_proof.L;
    let R = ip_proof.R;
    let mut s_inv = s.clone();
    s_inv.reverse();
    let y_inv = y.inverse().unwrap();
    let two_n : Vec<C::Scalar> = two_n_vec(n);
    for i in 0..G.len(){
        let j = i/ n as usize;
        let mut H_scalar = two_n[i%n as usize];
        H_scalar.mul_assign(&z_2_m[j]);
        let mut bs_inv = b;
        bs_inv.mul_assign(&s_inv[i]);
        H_scalar.sub_assign(&bs_inv);
        H_scalar.mul_assign(&y_i);
        y_i.mul_assign(&y_inv);
        H_scalar.add_assign(&z);
        H_scalars.push(H_scalar);
    }
    let H_term = multiscalar_multiplication(&H_scalars, &H); //Expensive!
    let A_term = A;
    let S_term = S.mul_by_scalar(&x);
    let mut B_scalar = tx;
    let mut ab = a;
    ab.mul_assign(&b);
    B_scalar.sub_assign(&ab);
    B_scalar.mul_assign(&w);
    let B_term = B.mul_by_scalar(&B_scalar);
    let mut minus_e_tilde = e_tilde;
    minus_e_tilde.negate();
    let B_tilde_scalar = minus_e_tilde;
    let B_tilde_term = B_tilde.mul_by_scalar(&B_tilde_scalar);
    let mut G_scalars = Vec::with_capacity(G.len());
    for i in 0..G.len() {
        let mut G_scalar = z;
        G_scalar.negate();
        let mut sa = s[i];
        sa.mul_assign(&a);
        G_scalar.sub_assign(&sa);
        G_scalars.push(G_scalar);
    }
    let G_term = multiscalar_multiplication(&G_scalars, &G); //Expensive!
    let L_term = multiscalar_multiplication(&u_sq, &L); //Expensive!
    let R_term = multiscalar_multiplication(&u_inv_sq, &R); //Expensive!

    let mut sum = A_term.plus_point(&S_term).plus_point(&B_term).plus_point(&B_tilde_term).plus_point(&G_term).plus_point(&H_term).plus_point(&L_term).plus_point(&R_term);
    

    let second = sum.is_zero_point();
    println!("Second check = {:?}", second);
    first && second 
}

fn verify_more_efficient<C: Curve>(transcript: &mut Transcript, n: u8, commitments: Vec<Commitment<C>>, proof: RangeProof<C>, mut G: Vec<C>, mut H: Vec<C>, B: C, B_tilde: C) -> bool{
    let m = commitments.len();
    for V in commitments.clone() {
        transcript.append_point(b"Vj", &V.0);
    }
    let A = proof.A;
    let S = proof.S;
    let T_1 = proof.T_1;
    let T_2 = proof.T_2;
    let tx = proof.tx;
    let tx_tilde = proof.tx_tilde;
    let e_tilde = proof.e_tilde;
    transcript.append_point(b"A", &A);
    transcript.append_point(b"S", &S);
    let y : C::Scalar = transcript.challenge_scalar::<C>(b"y");
    let z : C::Scalar = transcript.challenge_scalar::<C>(b"z");
    let mut z2 = z;
    z2.mul_assign(&z);
    let mut z3 = z2;
    z3.mul_assign(&z);
    transcript.append_point(b"T1", &T_1);
    transcript.append_point(b"T2", &T_2);
    let x : C::Scalar = transcript.challenge_scalar::<C>(b"x");
    let mut x2 = x;
    x2.mul_assign(&x);
    println!("verifier's x = {:?}", x);
    transcript.append_scalar::<C>(b"tx", &tx);
    transcript.append_scalar::<C>(b"tx_tilde", &tx_tilde);
    transcript.append_scalar::<C>(b"e_tilde", &e_tilde);
    let w : C::Scalar = transcript.challenge_scalar::<C>(b"w");
    //Calculate delta(x,y):
    let mut ip_1_y_nm = C::Scalar::zero();
    let mut yi = C::Scalar::one();
    for _ in 0..G.len() {
        ip_1_y_nm.add_assign(&yi);
        yi.mul_assign(&y);
    }
    let mut ip_1_2_n = C::Scalar::zero();
    let mut two_i = C::Scalar::one();
    for _ in 0..n as usize {
        ip_1_2_n.add_assign(&two_i);
        two_i.double();
    }
    let mut sum = C::Scalar::zero();
    let mut zj3 = z3;
    for _ in 0..m {
        sum.add_assign(&zj3);
        zj3.mul_assign(&z);
    } 
    sum.mul_assign(&ip_1_2_n);
    let mut delta_yz = z;
    delta_yz.sub_assign(&z2);
    delta_yz.mul_assign(&ip_1_y_nm);
    delta_yz.sub_assign(&sum);

    // //LHS of check equation 1:
    // let LHS = B.mul_by_scalar(&tx).plus_point(&B_tilde.mul_by_scalar(&tx_tilde));
    // let mut RHS = C::zero_point();
    // let mut zj2 = z2;
    // for j in 0..m {
    //     let Vj = commitments[j].0;
    //     // println!("V_{:?} = {:?}", j, Vj);
    //     RHS = RHS.plus_point(&Vj.mul_by_scalar(&zj2));
    //     zj2.mul_assign(&z);
    // }
    // RHS = RHS.plus_point(&B.mul_by_scalar(&delta_yz)).plus_point(&T_1.mul_by_scalar(&x)).plus_point(&T_2.mul_by_scalar(&x2));

    println!("--------------- VERIFICATION ----------------");
    // println!("LHS = {:?}", LHS);
    // println!("RHS = {:?}", RHS);
    // println!("Are they equal? {:?}", LHS == RHS);
    // println!("First check = {:?}", LHS.minus_point(&RHS).is_zero_point());
    
    let ip_proof = proof.ip_proof;
    let mut H_scalars : Vec<C::Scalar> = Vec::with_capacity(G.len());
    let mut y_i = C::Scalar::one();
    let mut z_2_m : Vec<C::Scalar> = Vec::with_capacity(m);
    let mut z_j = z;
    z_j.mul_assign(&z);
    for j in 0..m {
        z_2_m.push(z_j);
        z_j.mul_assign(&z);
    }
    let (u_sq, u_inv_sq, s) = verify_scalars(transcript, G.len(), &ip_proof.clone());
    let a = ip_proof.a;
    let b = ip_proof.b;
    let mut L = ip_proof.L;
    let mut R = ip_proof.R;
    let mut s_inv = s.clone();
    s_inv.reverse();
    let y_inv = y.inverse().unwrap();
    let two_n : Vec<C::Scalar> = two_n_vec(n);
    for i in 0..G.len(){
        let j = i/ n as usize;
        let mut H_scalar = two_n[i%n as usize];
        H_scalar.mul_assign(&z_2_m[j]);
        let mut bs_inv = b;
        bs_inv.mul_assign(&s_inv[i]);
        H_scalar.sub_assign(&bs_inv);
        H_scalar.mul_assign(&y_i);
        y_i.mul_assign(&y_inv);
        H_scalar.add_assign(&z);
        H_scalars.push(H_scalar);
    }
    let A_scalar = C::Scalar::one();
    let S_scalar = x;
    let c = w; // TODO: Shuld be generated randomly
    let mut T_1_scalar = c;
    T_1_scalar.mul_assign(&x);
    let mut T_2_scalar = T_1_scalar;
    T_2_scalar.mul_assign(&x);

    let mut V_scalars = Vec::with_capacity(m);
    let mut cz_i = c;
    cz_i.mul_assign(&z);
    cz_i.mul_assign(&z);
    for _ in 0..m {
        V_scalars.push(cz_i);
        cz_i.mul_assign(&z);
    }

    let mut B_scalar = tx;
    let mut ab = a;
    ab.mul_assign(&b);
    B_scalar.sub_assign(&ab);
    B_scalar.mul_assign(&w);
    let mut c_delta_minus_tx = delta_yz;
    c_delta_minus_tx.sub_assign(&tx);
    c_delta_minus_tx.mul_assign(&c);
    B_scalar.add_assign(&c_delta_minus_tx);
    let B_term = B.mul_by_scalar(&B_scalar);
    let mut minus_e_tilde = e_tilde;
    minus_e_tilde.negate();
    let mut B_tilde_scalar = minus_e_tilde;
    let mut ctx_tilde = tx_tilde;
    ctx_tilde.mul_assign(&c);
    B_tilde_scalar.sub_assign(&ctx_tilde);
    let B_tilde_term = B_tilde.mul_by_scalar(&B_tilde_scalar);
    let mut G_scalars = Vec::with_capacity(G.len());
    for i in 0..G.len() {
        let mut G_scalar = z;
        G_scalar.negate();
        let mut sa = s[i];
        sa.mul_assign(&a);
        G_scalar.sub_assign(&sa);
        G_scalars.push(G_scalar);
    }
    let mut Vjs : Vec<C> = commitments.iter().map(|x| x.0).collect();
    // let H_term = multiscalar_multiplication(&H_scalars, &H); //Expensive!
    // let A_term = A;
    // let S_term = S.mul_by_scalar(&x);
    // let G_term = multiscalar_multiplication(&G_scalars, &G); //Expensive!
    // let L_term = multiscalar_multiplication(&u_sq, &L); //Expensive!
    // let R_term = multiscalar_multiplication(&u_inv_sq, &R); //Expensive!
    // let V_term = multiscalar_multiplication(&V_scalars, &Vjs); //Expensive!
    // let T_1_term = T_1.mul_by_scalar(&T_1_scalar);
    // let T_2_term = T_2.mul_by_scalar(&T_2_scalar);
    // let mut sum = A_term.plus_point(&S_term).plus_point(&B_term).plus_point(&B_tilde_term).plus_point(&G_term).plus_point(&H_term).plus_point(&L_term).plus_point(&R_term).plus_point(&V_term).plus_point(&T_1_term).plus_point(&T_2_term);
    let mut all_scalars = vec![A_scalar];
    all_scalars.push(S_scalar);
    all_scalars.push(T_1_scalar);
    all_scalars.push(T_2_scalar);
    all_scalars.push(B_scalar);
    all_scalars.push(B_tilde_scalar);
    all_scalars.append(&mut V_scalars);
    all_scalars.append(&mut G_scalars);
    all_scalars.append(&mut H_scalars);
    let mut L_scalars = u_sq;
    let mut R_scalars = u_inv_sq;
    all_scalars.append(&mut L_scalars);
    all_scalars.append(&mut R_scalars);
    let mut all_points = vec![A];
    all_points.push(S);
    all_points.push(T_1);
    all_points.push(T_2);
    all_points.push(B);
    all_points.push(B_tilde);
    all_points.append(&mut Vjs);
    all_points.append(&mut G);
    all_points.append(&mut H);
    all_points.append(&mut L);
    all_points.append(&mut R);

    let sum2 = multiscalar_multiplication(&all_scalars, &all_points);
    // println!("len of msm vector = {}", all_scalars.len());


    // println!("Second check = {:?}", sum.is_zero_point());
    let b : bool = sum2.is_zero_point();
    println!(" check = {:?}", b);
    // println!("sum1==sum2? {:?}", sum==sum2);
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use bit_vec::BitVec;
    use std::time::{Duration, Instant};
    type SomeCurve = G1;
    type SomeField = Fr;

    #[test]
    fn test_repr(){
        let zero = SomeField::zero();
        let one = SomeField::one();
        let mut two = SomeField::one();
        two.add_assign(&one);
        let mut three = SomeField::one();
        three.add_assign(&two);
        let mut four = three;
        four.add_assign(&one);
        let mut five = three;
        five.add_assign(&two);
        println!("{:?}", one);
        println!("{:?}", two);
        println!("{:?}", three);

        let three_bv = vec![one, one];
        let two_n = vec![one, two];
        let ip = inner_product(&three_bv, &two_n);
        println!("three = {:?}", ip);

        let five_bv = vec![one, zero, one];
        let two_n = vec![one, two, four];
        let ip = inner_product(&five_bv, &two_n);
        println!("five = {:?}", ip);

        let seven = Fr::from_str("7").unwrap();
        let seven_repr = FrRepr::from(7);
        println!("{:?}", seven);
        println!("{:?}", seven_repr);
        println!("{:?}", seven.into_repr());
        println!("{:?}", Fr::from_repr(seven_repr));

        
        let v = 10;
        let n = 4;
        let (a_L, a_R) = a_L_a_R::<SomeField>(v, n);
        let two_n = two_n_vec(n);
        let ip = inner_product(&a_L, &two_n);
        println!("v = {:?}", ip);
        println!("a_L o a_R = {:?}", mul_vectors(&a_L, &a_R));
        assert!(true);
    }

    

    #[test]
    fn test_bv(){
        let a   = 256 as u32;
        let a   = 3 as u64;
        let bytes = a.to_be_bytes();
        
        let avec = BitVec::from_bytes(&bytes);
        for i in avec {
            let mut b = 0;
            if i {
                b = 1;
            }
            print!("{:?}", b);
        }
        print!("\n\n");
        for i in 0..10 {
            print!("{:?}", ith_bit(a, i));
        }


        println!("\n\n");

    }

    #[test]
    fn test_vector_com(){
        
        let rng = &mut thread_rng();
        let mut G = vec![];
        let mut values = vec![];
        let n = 3;
        let h = SomeCurve::generate(rng);
        // let r = SomeCurve::generate_scalar(rng);
        let r = Randomness::generate(rng);
        for _ in 0..n {
            let g = SomeCurve::generate(rng);
            let value = Value::new(SomeCurve::generate_scalar(rng));
            G.push(g);
            values.push(value);
        }
        let keys = CommitmentKey(G, h);
        let com = keys.hide(&values[..], &r);

        // let mut vv = &values[0];
        // vv.mul_assign(&SomeField::one());
        // println!("{:?}", vv);

        // let values_deref : Vec<_> = values.iter().map(|x| x.value).collect();
        // println!("{:?}", values_deref);
        // println!("\n\n\n");
        // println!("{:?}", *(&values[0..2]));
        println!("{:?}", com);
    }

    #[test]
    fn test_prove(){
        // Test for n = m = 4
        let rng = &mut thread_rng();
        let n = 32;
        let m = 16;
        let nm = (n as usize)*(m as usize);
        let mut G = Vec::with_capacity(nm);
        let mut H = Vec::with_capacity(nm);

        for i in 0..(nm) {
            let g = SomeCurve::generate(rng);
            let h = SomeCurve::generate(rng);

            G.push(g);
            H.push(h);
        }
        let B = SomeCurve::generate(rng);
        let B_tilde = SomeCurve::generate(rng);

        //Some numbers in [0, 2^n):
        let v_vec : Vec<u64> = vec![7,4,255,15,2,15,4294967295,4,
                                    4,5,6,8,12,13,10,8
                                    // ,7,4,15,15,2,15,5,4,4,5,6,8,12,13,10,8
                                    // ,7,4,15,15,2,15,5,4,4,5,6,8,12,13,10,8
                                    // ,7,4,15,15,2,15,5,4,4,5,6,8,12,13,10,8
                                    // ,7,4,15,15,2,15,5,4,4,5,6,8,12,13,10,8
                                    // ,7,4,15,15,2,15,5,4,4,5,6,8,12,13,10,8
                                    // ,7,4,15,15,2,15,5,4,4,5,6,8,12,13,10,8
                                    // ,7,4,15,15,2,15,5,4,4,5,6,8,12,13,10,8
                                    ];
        let mut transcript = Transcript::new(&[]);
        println!("Prove that all numbers in {:?} are in [0, 2^{:?})", v_vec.clone(), n);
        let now = Instant::now();
        println!("Proving..");
        let (commitments, proof) = prove(n, m, v_vec.clone(), G.clone(), H.clone(), B, B_tilde, rng, &mut transcript);
        println!("Prove done! Proving time: {} ms", now.elapsed().as_millis());
        
        let now = Instant::now();
        println!("Verifying..");
        let mut transcript = Transcript::new(&[]);
        let b = verify(&mut transcript, n, commitments.clone(), proof.clone(), G.clone(), H.clone(), B, B_tilde);
        // println!("P_prime equal? {}", P_prime_prover == P_prime_verifier);
        println!("Verification done! time: {} ms", now.elapsed().as_millis());

        //Testing the (slightly) more efficient verifier:
        println!("Verifying (slightly) more efficiently..");
        let now = Instant::now();
        let mut transcript = Transcript::new(&[]);
        let b = verify_efficient(&mut transcript, n, commitments.clone(), proof.clone(), G.clone(), H.clone(), B, B_tilde);
        println!("Verification done! time: {} ms", now.elapsed().as_millis());
        // println!("Efficient verifier's output = {:?}", b);

         //Testing the even more efficient verifier:
         println!("Verifying even more efficiently..");
         let now = Instant::now();
         let mut transcript = Transcript::new(&[]);
         let b = verify_more_efficient(&mut transcript, n, commitments.clone(), proof.clone(), G.clone(), H.clone(), B, B_tilde);
         println!("Verification done! time: {} ms", now.elapsed().as_millis());
         // println!("Efficient verifier's output = {:?}", b);


        //CHEATING prover:
        println!("\n\n --------------------- CHEATING PROVER -----------------------");
        let mut transcript = Transcript::new(&[]);
        println!("Prove that all numbers in {:?} are in [0, 2^{:?})", v_vec.clone(), n);
        let now = Instant::now();
        println!("Proving..");
        let (commitments, proof) = cheat_prove(n, m, v_vec, G.clone(), H.clone(), B, B_tilde, rng, &mut transcript);
        println!("Prove done! Proving time: {} ms", now.elapsed().as_millis());
        
        println!("Verifying more efficiently..");
        let now = Instant::now();
        let mut transcript = Transcript::new(&[]);
        let b = verify_efficient(&mut transcript, n, commitments.clone(), proof.clone(), G.clone(), H.clone(), B, B_tilde);
        println!("Verification done! time: {} ms", now.elapsed().as_millis());
        println!("Verifying even more efficiently..");
        let now = Instant::now();
        let mut transcript = Transcript::new(&[]);
        let b = verify_more_efficient(&mut transcript, n, commitments.clone(), proof.clone(), G.clone(), H.clone(), B, B_tilde);
        println!("Verification done! time: {} ms", now.elapsed().as_millis());

        // println!("Prover's P' = {:?}", P_prime_prover.into_affine());
        // println!("Verifier's P' = {:?}", P_prime_verifier.into_affine());
        // println!("hello");
    }
}
