# LaserVision: Real-time Laser Measurement with a Webcam

This is the software component in building a highly accurate measurement device primarily used in a similar way to a surface plate. Sofar measurements are attainable between 0.5um - 2um in repeated measurements.

![alt text](images/GUI.png)

Video on the usage of this tool:
[![Video](http://img.youtube.com/vi/hnHjrz_inQU/0.jpg)](http://www.youtube.com/watch?v=hnHjrz_inQU)


Because of how image sensor technology has progressed, we can use that  as the basis of a highly accurate measurement device. This tool works by reading the laser sensor’s intensity values on a webcam with its camera lens taken off.  An image sensor off a typical $15 webcam is about 3mm wide and has a resolution of FHD (1920x1080). This means that each light sensor is in the range of 1-5 microns in size.

The camera sensor is mounted at 90 degree angle so the wide direction is vertical. This gives a high coverage to sense the beam and also rescues power source based noise like PWM noise in switching power supplies.

The laser beam can be a point or a preferred horizontal line (like in a self leveling laser that I am using). We take the mean of each row (ex 1080 pixels wide) reshaping the 2D image into a 1D array. We then take this 1D array and fit a gaussian curve to it to find the center point. Finally converting the pixel position into physical height from calculations based on the physical size of the sensor to its resolution.

Further improvements reaching sub-pixel noise can be done by multisampling the results, smoothing out the luminosity noise in the 1D array, and removal of outliers by percentage.

This tool is based on the hard work of this original [Java tool](https://github.com/betzuka/laserlevel) but using a different base language Python and further changes to improve its accuracy and usability, and sample speed.

# Install

Binary installer can be downloaded on PyPI
```shell
pip install laser-level-webcam==1.0.0
```
and than running the command:
```shell
laser-level-webcam
```

# Build from source

Follow these steps to install the Python program:

### 1. Clone the Repository

Start by cloning the repository containing the Python program to your local machine. Open a terminal and run the following command:

```bash
git clone https://github.com/bhowiebkr/laser-level-webcam.git
```

### 2. Create a Virtual Environment

Navigate to the cloned repository directory using the cd command:

```bash
cd laser-level-webcam
```

Next, create a virtual environment for isolating the program's dependencies. Run the following commands:

2a. if you are missing virtualenv, install it with pip
```bash
pip install virtualenv
```
2b. Setup the virtualenv:
```bash
virtualenv venv
```

### 3. Activate the Virtual Environment

Activate the virtual environment to start using it.
```bash
.\venv\Scripts\activate
```

### 4. Install Required Packages

The program's required packages are listed in the requirements.txt file. This will install the packages into the virtual environmnent setup in step 2. To install these packages, run the following command:

```bash
pip install -r requirements.txt
```
This command uses `pip` to install the packages specified in the requirements.txt file.

### 5. Run the Python Program
After installing the required packages, you are ready to run the Python program:

```bash
python.exe laser_level_tool/main.py
```
We run the `main.py` file in the `laser_level_tool` directory

## Usage

I’ll make a video explaining this tool and the workflow in greater detail, but in the meantime, work from the left to the right of the tool. Some helpful tips:

- About 100 sub-samples. I tried up to 1000 sub-samples and didn’t find much more of a difference.
- remove 50% of the outliers
- crank the smoothing way up
- let the tool stay on for at least 5 minutes so the webcam sensor stays at a consistent temp. It will drift while getting up to temp. It might be best to test this on your own sensor by taking the same measurement from a cold start and time how long it takes before the samples stop drifting.
- Make sure nobody is walking around.
- Don’t stand when taking a sample because it’ll pick up your leg muscles. Sit and don’t move.
- Disable auto exposure and auto color temp. Disable anything that says auto in the device config with the extra attribute button (bottom left)
- If using a self leveling laser level. Anything will make the beam wobble. Look out for sources like people, computers/electronics with fans, HVAC systems, etc.


## License

This project is licensed under the MIT License. See the `LICENSE` file for details.
